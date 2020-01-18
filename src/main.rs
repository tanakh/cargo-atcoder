use anyhow::{anyhow, Result};
use chrono::{DateTime, Local};
use console::Style;
use futures::future::FutureExt;
use futures::{join, pending, select};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use serde_derive::Deserialize;
use sha2::digest::Digest;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{
    cmp::min,
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use structopt::StructOpt;
use tokio::time::delay_for;

// use termion::event::{Event, Key};
// use termion::input::TermRead;

mod atcoder;

use atcoder::*;

#[derive(Debug, Deserialize)]
struct Config {
    template: String,
    rustc_version: String,
}

fn read_config() -> Result<Config> {
    let config_path = dirs::config_dir()
        .ok_or(anyhow!("Failed to get config directory"))?
        .join("cargo-atcoder.toml");
    let s = std::fs::read_to_string(&config_path)
        .map_err(|_| anyhow!("Cannot read file: {}", config_path.display()))?;
    let config: Config = toml::from_str(&s)?;

    // dbg!(&config);
    Ok(config)
}

fn session_file() -> PathBuf {
    dirs::cache_dir()
        .unwrap()
        .join("cargo-atcoder/session.json")
}

#[derive(StructOpt)]
struct NewOpt {
    /// Contest ID (e.g. abc123)
    contest_id: String,

    /// Number of problems
    #[structopt(short, long, default_value = "6")]
    num_problems: usize,
}

fn new_project(opt: NewOpt) -> Result<()> {
    assert!(opt.num_problems <= 26);

    let config = read_config()?;

    let dir = Path::new(&opt.contest_id);
    if dir.is_dir() || dir.is_file() {
        Err(anyhow!("Directory {} already exists", dir.display()))?;
    }

    // FIXME: use specified version via rustup
    let stat = Command::new("cargo")
        .arg("new")
        .arg(&opt.contest_id)
        .status()?;
    if !stat.success() {
        Err(anyhow!("Failed to create project: {}", &opt.contest_id))?;
    }

    fs::remove_file(dir.join("src").join("main.rs"))?;
    fs::create_dir(dir.join("src").join("bin"))?;

    fs::write(dir.join("rust-toolchain"), &config.rustc_version)?;

    for i in 0..opt.num_problems {
        fs::write(
            dir.join("src")
                .join("bin")
                .join(format!("{}.rs", ('a' as u8 + i as u8) as char)),
            &config.template,
        )?;
    }

    Ok(())
}

async fn login() -> Result<()> {
    let username = dialoguer::Input::<String>::new()
        .with_prompt("Username")
        .interact()?;

    let password = dialoguer::PasswordInput::new()
        .with_prompt("Password")
        .interact()?;

    let atc = AtCoder::new(&session_file())?;
    atc.login(&username, &password).await?;

    println!("Login succeeded.");

    Ok(())
}

#[derive(StructOpt)]
struct TestOpt {
    problem_id: String,
    /// Specify case number to test
    case_num: Option<usize>,
    /// Submit if test passed
    #[structopt(short, long)]
    submit: bool,
}

async fn test(opt: TestOpt) -> Result<()> {
    let conf = read_config()?;

    let atc = AtCoder::new(&session_file())?;
    // FIXME: check logined

    let problem_id = opt.problem_id;
    let contest_id = get_cur_contest_id()?;
    let contest_info = atc.contest_info(&contest_id).await?;

    let problem = contest_info.problem(&problem_id).ok_or(anyhow!(
        "Problem `{}` is not contained in this contest",
        &problem_id
    ))?;

    let test_cases = atc.test_cases(&problem.url).await?;

    let res = test_samples(&problem_id, &test_cases)?;
    if res {
        if opt.submit {
            let source = fs::read(format!("src/bin/{}.rs", problem_id))
                .map_err(|_| anyhow!("Failed to read {}.rs", problem_id))?;

            atc.submit(&contest_id, &problem_id, &String::from_utf8_lossy(&source))
                .await?;
        }
    }

    Ok(())
}

fn get_cur_contest_id() -> Result<String> {
    let manifest = cargo_toml::Manifest::from_path("Cargo.toml")?;
    let package = manifest.package.unwrap();
    Ok(package.name)
}

fn test_samples(problem_id: &str, test_cases: &[TestCase]) -> Result<bool> {
    let build_status = Command::new("cargo")
        .arg("build")
        .arg("--bin")
        .arg(&problem_id)
        .status()?;

    if !build_status.success() {
        return Ok(false);
    }

    let test_case_num = test_cases.len();

    println!("running {} tests", test_case_num);

    let mut fails = vec![];
    let green = Style::new().green();
    let red = Style::new().red();
    let cyan = Style::new().cyan();

    for (i, test_case) in test_cases.iter().enumerate() {
        let mut child = Command::new("cargo")
            .arg("run")
            .arg("-q")
            .arg("--bin")
            .arg(&problem_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(test_case.input.as_bytes())?;

        let output = child.wait_with_output()?;
        if !output.status.success() {
            println!("test sample {} ... {}", i + 1, red.apply_to("FAILED"));
            fails.push((i, false, output));
            continue;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        if stdout.trim() != test_case.output.trim() {
            println!("test sample {} ... {}", i + 1, red.apply_to("FAILED"));
            fails.push((i, true, output));
        } else {
            println!("test sample {} ... {}", i + 1, green.apply_to("ok"));
        }
    }
    println!();

    let fail_num = fails.len();

    if fail_num == 0 {
        println!("test_result: {}", green.apply_to("ok"));
        println!();
        return Ok(true);
    }

    for (case_no, exec_success, output) in fails {
        println!("---- sample {} ----", case_no + 1);

        if !exec_success {
            println!(
                "{}: exit code: {}",
                red.apply_to("runtime error"),
                output.status.code().unwrap_or_default(),
            );
            println!();

            if output.stdout.len() > 0 {
                println!("stdout:");
                print_lines(&String::from_utf8_lossy(&output.stdout));
                println!();
            }

            if output.stderr.len() > 0 {
                println!("stderr:");
                print_lines(&String::from_utf8_lossy(&output.stderr));
                println!();
            }
        } else {
            println!("{}:", cyan.apply_to("input"));
            print_lines(&test_cases[case_no].input);
            println!();

            println!("{}:", green.apply_to("expected output"));
            print_lines(&test_cases[case_no].output);
            println!();

            println!("{}:", red.apply_to("your output"));
            print_lines(&String::from_utf8_lossy(&output.stdout));
            println!();

            if output.stderr.len() > 0 {
                println!("stderr:");
                print_lines(&String::from_utf8_lossy(&output.stderr));
                println!();
            }
        }
    }

    println!(
        "test result: {}. {} passed; {} failed",
        red.apply_to("FAILED"),
        test_case_num - fail_num,
        fail_num
    );
    println!();

    Ok(false)
}

fn print_lines(s: &str) {
    for (i, line) in s.lines().enumerate() {
        println!("{:6} | {}", i + 1, line);
    }
}

// use termion::raw::IntoRawMode;
// use tui::backend::TermionBackend;
// use tui::layout::{Constraint, Direction, Layout};
// use tui::style::{Color, Modifier, Style};
// use tui::widgets::{Block, Borders, Widget};
// use tui::Terminal;

async fn watch() -> Result<()> {
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

    let conf = read_config()?;

    let atc = AtCoder::new(&session_file())?;

    // FIXME: check logined

    let manifest = cargo_toml::Manifest::from_path("Cargo.toml")?;
    let package = manifest.package.unwrap();
    let contest_id = package.name;

    let atc = Arc::new(atc);

    // let submission_fut = {
    //     let atc = atc.clone();
    //     let contest_id = contest_id.clone();
    //     tokio::spawn(async move { watch_submission_status(&atc, &contest_id).await })
    // };

    let file_watcher_fut = {
        let atc = atc.clone();
        let contest_id = contest_id.clone();
        tokio::spawn(async move { watch_filesystem(&atc, &contest_id).await })
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

// async fn watch_submission_status(atc: &AtCoder, contest_id: &str) -> Result<()> {}

async fn watch_filesystem(atc: &AtCoder, contest_id: &str) -> Result<()> {
    let contest_info = atc.contest_info(&contest_id).await?;

    let (tx, rx) = channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx, Duration::from_millis(150))?;
    let rx = Arc::new(Mutex::new(rx));

    let cwd = std::env::current_dir()?;
    let src_dir = cwd.join("src/bin");

    watcher.watch(&src_dir, RecursiveMode::Recursive)?;

    let mut file_hash = BTreeMap::<String, _>::new();

    loop {
        let rx = rx.clone();
        let src_dir = src_dir.clone();
        let pb = tokio::task::spawn_blocking(move || -> Option<PathBuf> {
            if let DebouncedEvent::Write(pb) = rx.lock().unwrap().recv().unwrap() {
                let pb = pb.canonicalize().ok()?;
                let r = pb.strip_prefix(&src_dir).ok()?;
                if r.extension()? == "rs" {
                    return Some(r.to_owned());
                }
            }
            None
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

        let source = fs::read(format!("src/bin/{}.rs", problem_id))
            .map_err(|_| anyhow!("Failed to read {}.rs", problem_id))?;
        let hash = sha2::Sha256::digest(&source);

        if file_hash.get(&problem_id) == Some(&hash) {
            continue;
        }

        file_hash.insert(problem_id.clone(), hash);

        let test_cases = atc.test_cases(&problem.url).await?;
        let test_passed = test_samples(&problem_id, &test_cases)?;

        if !test_passed {
            continue;
        }

        // atc.submit(&contest_id, &problem_id, &String::from_utf8_lossy(&source))
        //     .await?;
    }
}

async fn info() -> Result<()> {
    let atc = AtCoder::new(&session_file())?;

    if let Some(username) = atc.username().await? {
        println!("Logged in as {}.", username);
    } else {
        println!("Not logged in.");
    }

    Ok(())
}

async fn status() -> Result<()> {
    let atc = AtCoder::new(&session_file())?;
    let contest_id = get_cur_contest_id()?;

    let m = Arc::new(MultiProgress::new());

    let join_fut = tokio::task::spawn_blocking({
        let m = m.clone();
        move || loop {
            m.join();
            std::thread::sleep_ms(50);
        }
    });

    let update_fut = tokio::task::spawn(async move {
        let mut dat = BTreeMap::new();

        let spinner_style = ProgressStyle::default_spinner().template("{prefix} {spinner} {msg}");

        let bar_style = ProgressStyle::default_bar()
            .template("{prefix} [{bar:30.cyan/blue}] {pos:>2}/{len:2} {msg}")
            .progress_chars(">=");

        let finish_style = ProgressStyle::default_spinner().template("{prefix} {msg}");

        let green = Style::new().green();
        let red = Style::new().red();

        loop {
            let mut results = atc.submission_status(&contest_id).await.unwrap();
            results.sort_by_key(|r| r.date);
            for result in results {
                let pb = dat.entry(result.id).or_insert_with(|| {
                    let pb = ProgressBar::new_spinner().with_style(spinner_style.clone());
                    pb.set_prefix(&format!(
                        "{} | {:20} | {:15} |",
                        DateTime::<Local>::from(result.date).format("%Y-%m-%d %H:%M:%S"),
                        &result.problem_name[0..min(20, result.problem_name.len())],
                        &result.language[0..min(15, result.language.len())],
                    ));
                    (pb, true)
                });

                match result.status {
                    StatusCode::Waiting(code) => {
                        pb.0.set_style(spinner_style.clone());
                        match code {
                            WaitingCode::WaitingForJudge => {
                                pb.0.set_message("Waiting for judge...")
                            }
                            WaitingCode::WaitingForRejudge => {
                                pb.0.set_message("Waiting for rejudge...")
                            }
                        }
                    }

                    StatusCode::Progress(cur, total, code) => {
                        pb.0.set_style(bar_style.clone());
                        pb.0.set_length(total as _);
                        pb.0.set_position(cur as _);
                        if let Some(code) = code {
                            let msg = code.short_msg();
                            pb.0.set_message(&format!(
                                "{}",
                                if code.accepted() {
                                    green.apply_to(&msg)
                                } else {
                                    red.apply_to(&msg)
                                }
                            ));
                        } else {
                            pb.0.set_message("");
                        }
                    }

                    StatusCode::Done(code) => {
                        if pb.1 {
                            let msg = code.long_msg();
                            let mut stat = format!(
                                "{} ({})",
                                if code.accepted() {
                                    green.apply_to(&msg)
                                } else {
                                    red.apply_to(&msg)
                                },
                                result.score
                            );
                            let space = 30 - console::measure_text_width(&stat);
                            for _ in 0..space {
                                stat += " ";
                            }
                            pb.0.set_style(finish_style.clone());
                            pb.0.finish_with_message(&format!(
                                "{}{}",
                                stat,
                                if let (Some(rt), Some(mm)) = (result.run_time, result.memory) {
                                    format!(" | {:>7} | {}", rt, mm)
                                } else {
                                    "".to_owned()
                                }
                            ));
                            pb.1 = false;
                        }
                    }
                }
            }

            let refresh_rate = 100;

            for _ in 0..3000 / refresh_rate {
                for (_, (pb, live)) in dat.iter() {
                    if *live {
                        pb.tick();
                    }
                }
                delay_for(Duration::from_millis(refresh_rate)).await;
            }
        }
    });

    let _ = join!(join_fut, update_fut);
    Ok(())
}

#[derive(StructOpt)]
enum Opt {
    #[structopt(name = "atcoder")]
    AtCoder(OptAtCoder),
}

#[derive(StructOpt)]
enum OptAtCoder {
    New(NewOpt),
    Login,
    Logout,
    Info,
    Test(TestOpt),
    Submit,
    Watch,
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    let Opt::AtCoder(opt) = Opt::from_args();

    use OptAtCoder::*;
    match opt {
        New(opt) => new_project(opt),
        Login => login().await,
        Logout => unimplemented!(),
        Info => info().await,
        Test(opt) => test(opt).await,
        Submit => unimplemented!(),
        Watch => watch().await,
        Status => status().await,
    }
}
