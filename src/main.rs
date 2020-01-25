use anyhow::{anyhow, Result};
use bytesize::ByteSize;
use chrono::{DateTime, Local};
use console::Style;
use futures::{future::FutureExt, join, select};
use handlebars::Handlebars;
use if_chain::if_chain;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use sha2::digest::Digest;
use std::{
    cmp::{max, min},
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::channel,
        Arc, Mutex,
    },
    time::Duration,
};
use structopt::StructOpt;
use tokio::time::delay_for;

// use termion::event::{Event, Key};
// use termion::input::TermRead;

mod atcoder;
mod config;

use atcoder::*;
use config::{read_config, Config};

fn session_file() -> PathBuf {
    dirs::cache_dir()
        .unwrap()
        .join("cargo-atcoder/session.json")
}

#[derive(StructOpt)]
struct NewOpt {
    /// Contest ID (e.g. abc123)
    contest_id: String,

    /// Create src/bin/<NAME>.rs without retrieving actual problem IDs
    #[structopt(short, long, value_name("NAME"))]
    bins: Vec<String>,
}

async fn new_project(opt: NewOpt) -> Result<()> {
    let config = read_config()?;

    let bins = if opt.bins.is_empty() {
        let atc = AtCoder::new(&session_file())?;

        match atc.contest_info(&opt.contest_id).await {
            Ok(info) => info.problem_ids_lowercase(),
            Err(err) => if_chain! {
                if let Some(reqwest_err) = err.downcast_ref::<reqwest::Error>();
                if reqwest_err.status().map(Into::into) == Some(404);
                then {
                    atc.problem_ids_of_rated_contest(&opt.contest_id)
                        .await?
                        .map(|ss| ss.iter().map(|s| s.to_lowercase()).collect())
                        .ok_or_else(|| {
                            err.context(
                                "could not find problem names. please specify names with `--bins`",
                            )
                        })?
                } else {
                    return Err(err);
                }
            },
        }
    } else {
        opt.bins
    };

    let dir = Path::new(&opt.contest_id);
    if dir.is_dir() || dir.is_file() {
        return Err(anyhow!("Directory {} already exists", dir.display()));
    }

    let stat = Command::new("cargo")
        .arg("new")
        .arg(&opt.contest_id)
        .status()?;
    if !stat.success() {
        return Err(anyhow!("Failed to create project: {}", &opt.contest_id));
    }

    // fs::write(dir.join("rust-toolchain"), &config.rustc_version)?;

    fs::remove_file(dir.join("src").join("main.rs"))?;
    fs::create_dir(dir.join("src").join("bin"))?;

    for bin in bins {
        fs::write(
            dir.join("src").join("bin").join(bin).with_extension("rs"),
            &config.project.template,
        )?;
    }

    let toml_file = dir.join("Cargo.toml");
    let mut manifest: BTreeMap<String, toml::Value> =
        toml::from_str(&fs::read_to_string(&toml_file)?)?;

    manifest.insert("dependencies".to_string(), config.dependencies.into());

    manifest.insert("profile".to_string(), {
        let mut m = BTreeMap::new();
        m.insert("release".to_string(), config.profile.release.clone());
        m.into()
    });

    fs::write(toml_file, toml::to_string_pretty(&manifest)?)?;

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

fn clear_session() -> Result<()> {
    let path = session_file();
    if path.is_file() {
        fs::remove_file(&path)?;
    }
    Ok(())
}

#[derive(StructOpt)]
struct TestOpt {
    /// Problem ID (e.g. a, b, ...)
    problem_id: String,
    /// Specify case number to test (e.g. 1, 2, ...)
    #[structopt(conflicts_with = "custom")]
    case_num: Vec<usize>,
    /// Use custom case from stdin
    #[structopt(short, long, conflicts_with = "case_num")]
    custom: bool,
    /// Submit if test passed
    #[structopt(short, long)]
    submit: bool,
    /// Use --release flag to compile
    #[structopt(long)]
    release: bool,
    /// Use verbose output
    #[structopt(short, long)]
    verbose: bool,
}

async fn test(opt: TestOpt) -> Result<()> {
    let atc = AtCoder::new(&session_file())?;
    let problem_id = opt.problem_id;
    let contest_id = get_cur_contest_id()?;
    let contest_info = atc.contest_info(&contest_id).await?;

    let problem = contest_info
        .problem(&problem_id)
        .ok_or_else(|| anyhow!("Problem `{}` is not contained in this contest", &problem_id))?;

    if opt.custom {
        return test_custom(&problem_id, opt.release);
    }

    let test_cases = atc.test_cases(&problem.url).await?;

    for &cn in opt.case_num.iter() {
        if cn == 0 || cn > test_cases.len() {
            return Err(anyhow!(
                "Case num {} is not found in problem {} samples",
                cn,
                problem_id
            ));
        }
    }

    let mut tcs = vec![];
    for (i, tc) in test_cases.into_iter().enumerate() {
        if opt.case_num.is_empty() || opt.case_num.contains(&(i + 1)) {
            tcs.push((i, tc));
        }
    }

    let passed = test_samples(&problem_id, &tcs, opt.release, opt.verbose)?;
    if passed && opt.submit {
        let source = fs::read(format!("src/bin/{}.rs", problem_id))
            .map_err(|_| anyhow!("Failed to read {}.rs", problem_id))?;

        atc.submit(&contest_id, &problem_id, &String::from_utf8_lossy(&source))
            .await?;
    }

    Ok(())
}

fn get_cur_contest_id() -> Result<String> {
    let manifest: toml::Value = toml::from_str(&fs::read_to_string("Cargo.toml")?)?;
    Ok(manifest["package"]["name"].as_str().unwrap().to_owned())
}

fn test_samples(
    problem_id: &str,
    test_cases: &[(usize, TestCase)],
    release: bool,
    verbose: bool,
) -> Result<bool> {
    let build_status = Command::new("cargo")
        .arg("build")
        .args(if release { vec!["--release"] } else { vec![] })
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

    for &(i, ref test_case) in test_cases.iter() {
        let mut child = Command::new("cargo")
            .arg("run")
            .args(if release { vec!["--release"] } else { vec![] })
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
            if verbose && !output.stderr.is_empty() {
                println!("stderr:");
                print_lines(&String::from_utf8_lossy(&output.stderr));
                println!();
            }
        }
    }
    println!();

    let fail_num = fails.len();

    for (case_no, exec_success, output) in fails {
        println!("---- sample {} ----", case_no + 1);

        if !exec_success {
            println!(
                "{}: exit code: {}",
                red.apply_to("runtime error"),
                output.status.code().unwrap_or_default(),
            );
            println!();

            if !output.stdout.is_empty() {
                println!("stdout:");
                print_lines(&String::from_utf8_lossy(&output.stdout));
                println!();
            }

            if !output.stderr.is_empty() {
                println!("stderr:");
                print_lines(&String::from_utf8_lossy(&output.stderr));
                println!();
            }
        } else {
            let tc = &test_cases.iter().find(|r| r.0 == case_no).unwrap().1;

            println!("{}:", cyan.apply_to("input"));
            print_lines(&tc.input);
            println!();

            println!("{}:", green.apply_to("expected output"));
            print_lines(&tc.output);
            println!();

            println!("{}:", red.apply_to("your output"));
            print_lines(&String::from_utf8_lossy(&output.stdout));
            println!();

            if !output.stderr.is_empty() {
                println!("stderr:");
                print_lines(&String::from_utf8_lossy(&output.stderr));
                println!();
            }
        }
    }

    if fail_num == 0 {
        println!("test_result: {}", green.apply_to("ok"));
        println!();
        Ok(true)
    } else {
        println!(
            "test result: {}. {} passed; {} failed",
            red.apply_to("FAILED"),
            test_case_num - fail_num,
            fail_num
        );
        println!();
        Ok(false)
    }
}

fn test_custom(problem_id: &str, release: bool) -> Result<()> {
    let build_status = Command::new("cargo")
        .arg("build")
        .args(if release { vec!["--release"] } else { vec![] })
        .arg("--bin")
        .arg(&problem_id)
        .status()?;

    if !build_status.success() {
        return Err(anyhow!("Build failed"));
    }

    println!("input test case:");

    let red = Style::new().red();
    let cyan = Style::new().cyan();

    let child = Command::new("cargo")
        .arg("run")
        .args(if release { vec!["--release"] } else { vec![] })
        .arg("-q")
        .arg("--bin")
        .arg(&problem_id)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let output = child.wait_with_output()?;
    if !output.status.success() {
        println!(
            "{}: exit code: {}",
            red.apply_to("runtime error"),
            output.status.code().unwrap_or_default(),
        );
        println!();

        if !output.stdout.is_empty() {
            println!("stdout:");
            print_lines(&String::from_utf8_lossy(&output.stdout));
            println!();
        }

        if !output.stderr.is_empty() {
            println!("stderr:");
            print_lines(&String::from_utf8_lossy(&output.stderr));
            println!();
        }
    } else {
        println!("{}:", cyan.apply_to("your output"));
        print_lines(&String::from_utf8_lossy(&output.stdout));
        println!();

        if !output.stderr.is_empty() {
            println!("stderr:");
            print_lines(&String::from_utf8_lossy(&output.stderr));
            println!();
        }
    }
    println!();
    Ok(())
}

fn print_lines(s: &str) {
    for (i, line) in s.lines().enumerate() {
        println!("{:6} | {}", i + 1, line);
    }
}

#[derive(StructOpt)]
struct SubmitOpt {
    /// Problem ID (e.g. a, b, ...)
    problem_id: String,
    /// Force submit even if test fails
    #[structopt(short, long)]
    force: bool,
    /// Skip test
    #[structopt(long)]
    skip_test: bool,
    /// Submit via binary (overwrite config)
    #[structopt(long, conflicts_with = "source")]
    bin: bool,
    /// Submit source code directory (overwrite config)
    #[structopt(long, conflicts_with = "bin")]
    source: bool,
    /// Do no use upx unless available
    #[structopt(long)]
    no_upx: bool,
    /// Use --release on pre-test (submission always uses --release)
    #[structopt(long)]
    release: bool,
}

async fn submit(opt: SubmitOpt) -> Result<()> {
    let atc = AtCoder::new(&session_file())?;
    let config = read_config()?;

    let contest_id = get_cur_contest_id()?;
    let problem_id = opt.problem_id;
    let contest_info = atc.contest_info(&contest_id).await?;
    let problem = contest_info
        .problem(&problem_id)
        .ok_or_else(|| anyhow!("Problem `{}` is not contained in this contest", &problem_id))?;

    let test_passed = if opt.skip_test {
        true
    } else {
        let test_cases = atc
            .test_cases(&problem.url)
            .await?
            .into_iter()
            .enumerate()
            .collect::<Vec<_>>();
        test_samples(&problem_id, &test_cases, opt.release, false)?
    };

    if !test_passed && !opt.force {
        println!("Sample test failed. Did not submit.");
        return Ok(());
    }

    let via_bin = opt.bin || (config.atcoder.submit_via_binary && !opt.source);
    let source = if !via_bin {
        fs::read(format!("src/bin/{}.rs", problem_id))
            .map_err(|_| anyhow!("Failed to read {}.rs", problem_id))?
    } else {
        println!("Submitting via binary...");
        gen_binary_source(&problem_id, &config, opt.no_upx)?
    };

    atc.submit(&contest_id, &problem_id, &String::from_utf8_lossy(&source))
        .await?;
    println!();

    println!("Fetching submission result...");
    let atc = Arc::new(atc);
    let last_id = watch_submission_status(Arc::clone(&atc), &contest_id, true).await?;
    println!();

    if let Some(last_id) = last_id {
        let res = atc.submission_status_full(&contest_id, last_id).await?;
        if let Some(code) = res.result.status.result_code() {
            if !code.accepted() {
                println!("Submission detail:");
                println!();
                print_full_result(&res, false)?;
            }
        }
    }

    Ok(())
}

fn gen_binary_source(problem_id: &str, config: &Config, no_upx: bool) -> Result<Vec<u8>> {
    let source_code = fs::read_to_string(format!("src/bin/{}.rs", problem_id))
        .map_err(|_| anyhow!("Failed to read {}.rs", problem_id))?;

    let target = &config.profile.target;
    let binary_file = format!("target/{}/release/{}", target, problem_id);

    let _ = fs::remove_dir_all(format!("target/{}/release", target));

    let status = Command::new("cargo")
        .arg("build")
        .arg(format!("--target={}", target))
        .arg("--release")
        .arg("--bin")
        .arg(&problem_id)
        .arg("--quiet")
        .status()?;

    if !status.success() {
        return Err(anyhow!("Build failed"));
    }

    let size = ByteSize::b(get_file_size(&binary_file)?);
    println!("Built binary size: {}", size);

    let status = Command::new("strip").arg("-s").arg(&binary_file).status()?;
    if !status.success() {
        return Err(anyhow!("strip failed"));
    }

    let size = ByteSize::b(get_file_size(&binary_file)?);
    println!("Stripped binary size: {}", size);

    if let Ok(upx_path) = which::which("upx") {
        if !no_upx {
            println!("upx found. Use upx to compress binary.");
            let status = Command::new(upx_path)
                .arg("--best")
                .arg("-qq")
                .arg(&binary_file)
                .status()?;
            if !status.success() {
                return Err(anyhow!("upx failed"));
            }
            let size = ByteSize::b(get_file_size(&binary_file)?);
            println!("Compressed binary size: {}", size);
        }
    }

    let code = {
        let mut handlebars = Handlebars::new();
        handlebars.register_escape_fn(|s: &str| s.to_owned());

        let templ = include_str!("../data/binary_runner.rs.txt");
        handlebars.register_template_string("binary_runner", templ)?;

        let bin = fs::read(&binary_file)?;

        let mut data = BTreeMap::new();
        data.insert("BINARY", data_encoding::BASE64.encode(&bin));
        data.insert("SOURCE_CODE", source_code);
        data.insert(
            "HASH",
            data_encoding::HEXUPPER.encode(&sha2::Sha256::digest(&bin))[0..8].to_owned(),
        );

        handlebars.render("binary_runner", &data)?.trim().to_owned()
    };

    let size = ByteSize::b(code.len() as u64);
    println!("Bundled code size: {}", size);

    let size_limit = ByteSize::kib(512);

    if size > size_limit {
        println!("Code size limit exceeded: larger than {}", size_limit);
    }

    Ok(code.bytes().collect::<Vec<u8>>())
}

fn get_file_size(path: impl AsRef<Path>) -> Result<u64> {
    let meta = fs::metadata(path)?;
    Ok(meta.len())
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

    // let conf = read_config()?;

    let atc = AtCoder::new(&session_file())?;

    let contest_id = get_cur_contest_id()?;

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
        let test_cases = test_cases.into_iter().enumerate().collect::<Vec<_>>();
        let test_passed = test_samples(&problem_id, &test_cases, false, false)?;

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

async fn watch_submission_status(
    atc: Arc<AtCoder>,
    contest_id: &str,
    recent_only: bool,
) -> Result<Option<usize>> {
    let config = read_config()?;
    let cur_time = chrono::offset::Utc::now();

    let contest_id = contest_id.to_owned();
    let m = Arc::new(MultiProgress::new());
    let complete = Arc::new(AtomicBool::new(false));

    let join_fut = tokio::task::spawn_blocking({
        let m = m.clone();
        let complete = Arc::clone(&complete);
        move || {
            while !complete.load(Ordering::Relaxed) {
                m.join().unwrap();
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    });

    let complete_ = Arc::clone(&complete);
    let update_fut = tokio::task::spawn(async move {
        let mut dat = BTreeMap::new();

        let spinner_style =
            ProgressStyle::default_spinner().template("{prefix} {spinner:.cyan} {msg}");

        let bar_style = ProgressStyle::default_bar()
            .template("{prefix} [{bar:30.cyan/blue}] {pos:>2}/{len:2} {msg}")
            .progress_chars("=>.");

        let finish_style = ProgressStyle::default_spinner().template("{prefix} {msg}");

        let green = Style::new().green();
        let red = Style::new().red();

        let mut last_id;

        loop {
            let results = atc.submission_status(&contest_id).await?;
            let mut results = if !recent_only {
                results
            } else {
                results
                    .into_iter()
                    .filter(|r| (cur_time - r.date).num_seconds() <= 10 || !r.status.done())
                    .collect::<Vec<_>>()
            };
            results.sort_by_key(|r| r.date);

            last_id = results.iter().last().map(|r| r.id);

            let mut done = true;

            for result in results {
                let pb = dat.entry(result.id).or_insert_with(|| {
                    let pb = ProgressBar::new_spinner().with_style(spinner_style.clone());

                    // pb.set_prefix(&format!(
                    //     "{} | {:20} | {:15} |",
                    //     DateTime::<Local>::from(result.date).format("%Y-%m-%d %H:%M:%S"),
                    //     &result.problem_name[0..min(20, result.problem_name.len())],
                    //     &result.language[0..min(15, result.language.len())],
                    // ));

                    pb.set_prefix(&format!(
                        "{} | {:20} |",
                        DateTime::<Local>::from(result.date).format("%Y-%m-%d %H:%M:%S"),
                        &result.problem_name[0..min(20, result.problem_name.len())],
                    ));

                    (pb, true)
                });

                match result.status {
                    StatusCode::Waiting(code) => {
                        done = false;
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
                        done = false;
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
                        // TODO: show result breakdown on error
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

            if done && recent_only {
                complete.store(true, Ordering::Relaxed);
                break;
            }

            let refresh_rate = 100;
            let update_interval = max(1000, config.atcoder.update_interval);

            for _ in 0..update_interval / refresh_rate {
                for (_, (pb, live)) in dat.iter() {
                    if *live {
                        pb.tick();
                    }
                }
                delay_for(Duration::from_millis(refresh_rate)).await;
            }
        }

        complete_.store(true, Ordering::Relaxed);

        let ret: Result<Option<usize>> = Ok(last_id);
        ret
    });

    Ok(join!(join_fut, update_fut).1??)
}

#[derive(StructOpt)]
struct GenBinaryOpt {
    /// Problem ID to make binary
    problem_id: String,
    /// Do not use UPX even if it is available
    #[structopt(long)]
    no_upx: bool,
}

fn gen_binary(opt: GenBinaryOpt) -> Result<()> {
    let config = read_config()?;
    let src = gen_binary_source(&opt.problem_id, &config, opt.no_upx)?;
    let filename = format!("{}-bin.rs", opt.problem_id);
    fs::write(&filename, &src)?;
    println!("Wrote code to `{}`", filename);
    Ok(())
}

#[derive(StructOpt)]
struct ResultOpt {
    /// submission ID
    submission_id: usize,
    /// Use verbose output
    #[structopt(long, short)]
    verbose: bool,
}

async fn result(opt: ResultOpt) -> Result<()> {
    let atc = AtCoder::new(&session_file())?;
    let contest_id = get_cur_contest_id()?;
    let res = atc
        .submission_status_full(&contest_id, opt.submission_id)
        .await?;

    print_full_result(&res, opt.verbose)
}

fn print_full_result(res: &FullSubmissionResult, verbose: bool) -> Result<()> {
    let green = Style::new().green();
    let red = Style::new().red();
    let cyan = Style::new().cyan();

    println!("Submission ID: {}", cyan.apply_to(res.result.id));
    println!(
        "Date:          {}",
        DateTime::<Local>::from(res.result.date).format("%Y-%m-%d %H:%M:%S")
    );
    println!("Problem:       {}", res.result.problem_name);
    println!("Language:      {}", res.result.language);
    println!("Score:         {}", res.result.score);
    println!("Code length:   {}", res.result.code_length);

    let stat = if let Some(code) = res.result.status.result_code() {
        let msg = code.long_msg();
        format!(
            "{}",
            if code.accepted() {
                green.apply_to(&msg)
            } else {
                red.apply_to(&msg)
            },
        )
    } else {
        "N/A".to_string()
    };

    println!("Result:        {}", stat);
    println!(
        "Runtime:       {}",
        res.result
            .run_time
            .as_ref()
            .map(|r| r.as_str())
            .unwrap_or("N/A")
    );
    println!(
        "Memory:        {}",
        res.result
            .memory
            .as_ref()
            .map(|r| r.as_str())
            .unwrap_or("N/A")
    );

    if res
        .result
        .status
        .result_code()
        .map(|c| !c.accepted())
        .unwrap_or(false)
        && !res.cases.is_empty()
    {
        let mut mm = BTreeMap::<&ResultCode, usize>::new();
        for case in res.cases.iter() {
            if let Some(code) = case.result.result_code() {
                *mm.entry(code).or_default() += 1;
            }
        }

        println!();
        println!("Breakdown:");

        for (code, count) in mm {
            let msg = format!("{:25}", code.long_msg());
            let msg = format!(
                "{}",
                if code.accepted() {
                    green.apply_to(&msg)
                } else {
                    red.apply_to(&msg)
                },
            );

            println!("    * {}: {}", msg, count);
        }

        if verbose {
            println!();
            println!("All result:");

            for case in res.cases.iter() {
                let stat = if let Some(code) = case.result.result_code() {
                    let msg = format!("{:15}", code.long_msg());
                    format!(
                        "{}",
                        if code.accepted() {
                            green.apply_to(&msg)
                        } else {
                            red.apply_to(&msg)
                        },
                    )
                } else {
                    "N/A".to_string()
                };
                println!(
                    "    * {:20} {}, {}, {}",
                    case.name.clone() + ":",
                    stat,
                    case.run_time.clone().unwrap_or("N/A".to_string()),
                    case.memory.clone().unwrap_or("N/A".to_string())
                );
            }
        }
    }
    Ok(())
}

async fn status() -> Result<()> {
    let atc = AtCoder::new(&session_file())?;
    let contest_id = get_cur_contest_id()?;
    let atc = Arc::new(atc);
    watch_submission_status(atc, &contest_id, false).await?;
    Ok(())
}

#[derive(StructOpt)]
#[structopt(bin_name("cargo"))]
enum Opt {
    #[structopt(name = "atcoder")]
    AtCoder(OptAtCoder),
}

#[derive(StructOpt)]
enum OptAtCoder {
    /// Create a new project for specified contest
    New(NewOpt),
    /// Login to atcoder
    Login,
    // /// Logout from atcoder
    // Logout,
    /// Clear session data (cookie store in HTTP client)
    ClearSession,
    /// Show session information
    Info,
    /// Test sample cases
    Test(TestOpt),
    /// Submit solution
    Submit(SubmitOpt),
    /// Show submission result detail
    Result(ResultOpt),
    /// Generate rustified binary
    GenBinary(GenBinaryOpt),
    /// Show submission status
    Status,
    /// [WIP] Watch filesystem for automatic submission
    Watch,
}

#[tokio::main]
async fn main() -> Result<()> {
    let Opt::AtCoder(opt) = Opt::from_args();

    let _ = read_config()?; // for checking config syntax

    use OptAtCoder::*;
    match opt {
        New(opt) => new_project(opt).await,
        Login => login().await,
        // Logout => unimplemented!(),
        ClearSession => clear_session(),
        Info => info().await,
        Test(opt) => test(opt).await,
        Submit(opt) => submit(opt).await,
        Result(opt) => result(opt).await,
        GenBinary(opt) => gen_binary(opt),
        Status => status().await,
        Watch => watch().await,
    }
}
