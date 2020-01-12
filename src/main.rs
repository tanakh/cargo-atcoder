use anyhow::{anyhow, Result};
use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use serde_derive::Deserialize;
use sha2::digest::Digest;
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::time::Duration;
use std::{fs, path::Path};
use structopt::StructOpt;

mod atcoder;

use atcoder::AtCoder;

#[derive(Debug, Deserialize)]
struct Config {
    template: String,
    rustc_version: String,

    atcoder_username: String,
    atcoder_password: String,
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

    let atc = AtCoder::new()?;
    atc.login(&username, &password).await?;

    println!("Login succeeded.");
    Ok(())
}

async fn watch() -> Result<()> {
    let conf = read_config()?;

    let atc = AtCoder::new()?;
    atc.login(&conf.atcoder_username, &conf.atcoder_password)
        .await?;

    // TODO: get contest id from Cargo.toml
    let contest_id = "abc123";

    let contest_info = atc.contest_info(contest_id).await?;

    // dbg!(&contest_info);

    // atc.submit(contest_id, "a", "HOGEHOGE").await?;

    // let test_cases = atc.test_cases(&contest_info.problems[0].url).await?;
    // dbg!(&test_cases);

    let (tx, rx) = channel();
    let mut watcher: RecommendedWatcher = Watcher::new(tx, Duration::from_millis(150))?;

    let cwd = std::env::current_dir()?;
    let src_dir = cwd.join("src/bin");

    watcher.watch(&src_dir, RecursiveMode::Recursive)?;

    let mut file_hash = BTreeMap::<String, _>::new();

    loop {
        let pb = if let DebouncedEvent::Write(pb) = rx.recv()? {
            let pb = if let Ok(pb) = pb.canonicalize() {
                pb
            } else {
                continue;
            };
            if let Ok(r) = pb.strip_prefix(&src_dir) {
                if r.extension() == Some("rs").map(AsRef::as_ref) {
                    r.to_owned()
                } else {
                    continue;
                }
            } else {
                continue;
            }
        } else {
            continue;
        };

        let problem_id = pb.file_stem().unwrap().to_string_lossy().into_owned();

        let problem = if let Some(problem) = contest_info.problem(&problem_id) {
            problem
        } else {
            eprintln!("Problem {} is not contained in this contest", &problem_id);
            continue;
        };

        let source = fs::read(format!("src/bin/{}.rs", problem_id))
            .map_err(|_| anyhow!("Failed to read {}.rs", problem_id))?;
        let hash = sha2::Sha256::digest(&source);

        if file_hash.get(&problem_id) == Some(&hash) {
            continue;
        }

        file_hash.insert(problem_id.clone(), hash);

        let build_status = Command::new("cargo")
            .arg("build")
            .arg("--bin")
            .arg(&problem_id)
            .status()?;

        if !build_status.success() {
            println!("Build failed");
            continue;
        }

        let test_cases = atc.test_cases(&problem.url).await?;
        let test_case_num = test_cases.len();
        let mut ok = 0;

        for (i, test_case) in test_cases.into_iter().enumerate() {
            let mut child = Command::new("cargo")
                .arg("run")
                .arg("-q")
                .arg("--bin")
                .arg(&problem_id)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()?;

            child
                .stdin
                .as_mut()
                .unwrap()
                .write_all(test_case.input.as_bytes())?;

            let output = child.wait_with_output()?;
            if !output.status.success() {
                println!(
                    "Failed to run: status code = {}",
                    output.status.code().unwrap_or_default()
                );
                continue;
            }

            let stdout = String::from_utf8_lossy(&output.stdout);

            if stdout.trim() != test_case.output.trim() {
                println!("Test case {} failed:", i + 1);
                println!("Input: {}", &test_case.input);
                println!("Expected: {}", &test_case.output);
                println!("Got: {}", &stdout);
                println!();
                continue;
            }

            ok += 1;
        }

        if ok != test_case_num {
            continue;
        }

        println!("Sample passed.");
        // atc.submit(&contest_id, &problem_id, &String::from_utf8_lossy(&source))
        //     .await?;
    }
}

fn submit_status(client: &reqwest::Client) -> Result<()> {
    unimplemented!()
}

#[derive(StructOpt)]
enum Opt {
    New(NewOpt),
    Login,
    Logout,
    Submit,
    Test,
    Watch,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Opt::from_args() {
        Opt::New(opt) => new_project(opt),
        Opt::Login => login().await,
        Opt::Watch => watch().await,
        _ => unimplemented!(),
    }
}
