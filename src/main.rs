use scraper::{Html, Selector};
use serde_derive::Deserialize;
use std::process::Command;
use std::{fs, path::Path};
use structopt::StructOpt;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const ATCODER_ENDPOINT: &str = "https://atcoder.jp";

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

async fn login() -> Result<()> {
    let client = reqwest::Client::builder().cookie_store(true).build()?;
    let resp = client
        .get(&format!("{}/login", ATCODER_ENDPOINT))
        .send()
        .await?
        .error_for_status()?;

    // let cookies = resp.cookies();

    let doc = resp.text().await?;

    let document = Html::parse_document(&doc);

    // dbg!(&document);

    let csrf_token = document
        .select(&Selector::parse("input[name=\"csrf_token\"]").unwrap())
        .next()
        .ok_or("cannot find csrf_token")?;

    // dbg!(&csrf_token);

    let csrf_token = csrf_token
        .value()
        .attr("value")
        .ok_or("cannot find csrf_token")?;

    let username = dialoguer::Input::<String>::new()
        .with_prompt("Username")
        .interact()?;

    let password = dialoguer::PasswordInput::new()
        .with_prompt("Password")
        .interact()?;

    let res = client
        .post(&format!("{}/login", ATCODER_ENDPOINT))
        .form(&[
            ("username", username.as_ref()),
            ("password", password.as_ref()),
            ("csrf_token", csrf_token),
        ])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;

    let res = Html::parse_document(&res);

    // On failure:
    // <div class="alert alert-danger alert-dismissible col-sm-12 fade in" role="alert">
    //   ...
    //   {error message}
    // </div>
    if let Some(err) = res
        .select(&Selector::parse("div.alert-danger").unwrap())
        .next()
    {
        Err(format!(
            "Login failed: {}",
            err.last_child().unwrap().value().as_text().unwrap().trim()
        ))?
    }

    // On success:
    // <div class="alert alert-success alert-dismissible col-sm-12 fade in" role="alert" >
    //     ...
    //     ようこそ、tanakh さん。
    // </div>
    if let Some(_) = res
        .select(&Selector::parse("div.alert-success").unwrap())
        .next()
    {
        println!("Login succeeded.");
        return Ok(());
    }

    Err("Login failed: Unknown error".into())
}

#[derive(StructOpt)]
enum Opt {
    New(NewOpt),
    Login,
    Logout,
    Submit,
    Test,
}

#[tokio::main]
async fn main() -> Result<()> {
    match Opt::from_args() {
        Opt::New(opt) => new_project(opt),
        Opt::Login => login().await,
        _ => unimplemented!(),
    }
}
