use serde_derive::Deserialize;
use std::process::Command;
use std::{fs, path::Path};
use structopt::StructOpt;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Deserialize)]
struct Config {
    template: String,
    rustc_version: String,
}

fn read_config() -> Result<Config> {
    let config_path = dirs::config_dir()
        .ok_or("Failed to get config directory")?
        .join("cargo-atcoder.toml");
    let s = std::fs::read_to_string(config_path)?;
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
        Err(format!("Directory {} already exists", dir.display()))?;
    }

    let stat = Command::new("cargo")
        .arg("new")
        .arg(&opt.contest_id)
        .status()?;
    if !stat.success() {
        Err(format!("Failed to create project: {}", &opt.contest_id))?;
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

#[derive(StructOpt)]
enum Opt {
    New(NewOpt),
    Login,
    Logout,
    Submit,
    Test,
}

fn main() -> Result<()> {
    match Opt::from_args() {
        Opt::New(opt) => new_project(opt),
        _ => unimplemented!(),
    }
}
