use std::{
    collections::BTreeMap,
    env, fs,
    path::PathBuf,
    sync::{mpsc::channel, Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use cargo_metadata::{Package, Target};
use futures::{select, FutureExt};
use sha2::Digest;
use structopt::StructOpt;

use crate::{
    atcoder::AtCoder,
    metadata::{self, MetadataExt, PackageExt},
    session_file, test_samples,
};

// use termion::raw::IntoRawMode;
// use tui::backend::TermionBackend;
// use tui::layout::{Constraint, Direction, Layout};
// use tui::style::{Color, Modifier, Style};
// use tui::widgets::{Block, Borders, Widget};
// use tui::Terminal;

#[derive(StructOpt, Debug)]
pub struct WatchOpt {
    /// [cargo] Package to watch
    #[structopt(short, long, value_name("SPEC"))]
    package: Option<String>,
    /// [cargo] Path to Cargo.toml
    #[structopt(long, value_name("PATH"))]
    manifest_path: Option<PathBuf>,
}

pub async fn watch(opt: WatchOpt) -> Result<()> {
    // let stdout = io::stdout().into_raw_mode()?;
    // let backend = TermionBackend::new(stdout);
    // let mut terminal = Terminal::new(backend)?;
    // terminal.clear();

    // terminal.draw(|mut f| {
    //     let size = f.size();
    //     Block::default()
    //         .title("Block")
    //         .borders(Borders::ALL)
    //         .render(&mut f, size);
    // })?;

    // let conf = read_config()?;

    let cwd = env::current_dir().with_context(|| "failed to get CWD")?;
    let metadata = metadata::cargo_metadata(opt.manifest_path.as_deref(), &cwd)?;
    let package = metadata.query_for_member(opt.package.as_deref())?.clone();
    let atc = AtCoder::new(&session_file()?)?;

    let atc = Arc::new(atc);

    // let submission_fut = {
    //     let atc = atc.clone();
    //     let contest_id = contest_id.clone();
    //     tokio::spawn(async move { watch_submission_status(&atc, &contest_id).await })
    // };

    let file_watcher_fut = {
        let atc = atc.clone();
        tokio::spawn(async move { watch_filesystem(&package, &atc).await })
    };

    // let ui_fut = {
    //     tokio::spawn(async move {
    //         for ev in io::stdin().events() {
    //             let ev = ev?;
    //             if ev == Event::Key(Key::Char('q')) || ev == Event::Key(Key::Ctrl('c')) {
    //                 break;
    //             }
    //         }

    //         let ret: Result<()> = Ok(());
    //         ret
    //     })
    // };

    select! {
        // _ = submission_fut.fuse() => (),
        _ = file_watcher_fut.fuse() => (),
        // _ = ui_fut.fuse() => (),
    };

    Ok(())
}

async fn watch_filesystem(package: &Package, atc: &AtCoder) -> Result<()> {
    use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};

    let contest_info = atc.contest_info(&package.name).await?;

    let (tx, rx) = channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx, Duration::from_millis(150))?;
    let rx = Arc::new(Mutex::new(rx));

    for Target { src_path, .. } in package.all_bins() {
        watcher.watch(src_path, RecursiveMode::NonRecursive)?;
    }

    let mut file_hash = BTreeMap::<String, _>::new();

    loop {
        let rx = rx.clone();
        let pb = tokio::task::spawn_blocking(move || -> Option<PathBuf> {
            if let DebouncedEvent::Write(pb) = rx.lock().unwrap().recv().unwrap() {
                let pb = pb.canonicalize().ok()?;
                let r = pb.strip_prefix(pb.parent()?).ok()?;
                Some(r.to_owned())
            } else {
                None
            }
        })
        .await?;

        if pb.is_none() {
            continue;
        }
        let pb = pb.unwrap();

        let problem_id = pb.file_stem().unwrap().to_string_lossy().into_owned();

        let problem = if let Some(problem) = contest_info.problem(&problem_id) {
            problem
        } else {
            eprintln!("Problem `{}` is not contained in this contest", &problem_id);
            continue;
        };

        let source = fs::read(&pb).with_context(|| format!("Failed to read {}", pb.display()))?;
        let hash = sha2::Sha256::digest(&source);

        if file_hash.get(&problem_id) == Some(&hash) {
            continue;
        }

        file_hash.insert(problem_id.clone(), hash);

        let test_cases = atc.test_cases(&problem.url).await?;
        let test_cases = test_cases.into_iter().enumerate().collect::<Vec<_>>();
        let test_passed = test_samples(package, &problem_id, &test_cases, false, false)?;

        if !test_passed {
            continue;
        }

        // atc.submit(&contest_id, &problem_id, &String::from_utf8_lossy(&source))
        //     .await?;
    }
}
