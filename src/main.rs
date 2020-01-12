use anyhow::{anyhow, Result};
use notify::{DebouncedEvent, RecommendedWatcher, RecursiveMode, Watcher};
use scraper::{Html, Selector};
use serde_derive::Deserialize;
use sha2::digest::Digest;
use std::collections::BTreeMap;
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::mpsc::channel;
use std::time::Duration;
use std::{fs, path::Path};
use structopt::StructOpt;

const ATCODER_ENDPOINT: &str = "https://atcoder.jp";

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

struct AtCoder {
    client: reqwest::Client,
}

async fn http_get(client: &reqwest::Client, url: &str) -> Result<String> {
    Ok(client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?)
}

impl AtCoder {
    fn new() -> Result<AtCoder> {
        Ok(AtCoder {
            client: reqwest::Client::builder().cookie_store(true).build()?,
        })
    }

    async fn login(&self, username: &str, password: &str) -> Result<()> {
        let doc = http_get(&self.client, &format!("{}/login", ATCODER_ENDPOINT)).await?;

        let document = Html::parse_document(&doc);

        // dbg!(&document);

        let csrf_token = document
            .select(&Selector::parse("input[name=\"csrf_token\"]").unwrap())
            .next()
            .ok_or(anyhow!("cannot find csrf_token"))?;

        // dbg!(&csrf_token);

        let csrf_token = csrf_token
            .value()
            .attr("value")
            .ok_or(anyhow!("cannot find csrf_token"))?;

        let res = self
            .client
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
            Err(anyhow!(
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
            return Ok(());
        }

        Err(anyhow!("Login failed: Unknown error"))
    }

    async fn contest_info(&self, contest_id: &str) -> Result<ContestInfo> {
        let doc = http_get(
            &self.client,
            &format!("{}/contests/{}/tasks", ATCODER_ENDPOINT, contest_id),
        )
        .await?;

        let doc = Html::parse_document(&doc);
        let sel_problem = Selector::parse("table tbody tr").unwrap();

        let mut problems = vec![];

        for row in doc.select(&sel_problem) {
            let sel_td = Selector::parse("td").unwrap();
            let mut it = row.select(&sel_td);
            let c1 = it.next().unwrap();
            let c2 = it.next().unwrap();
            let c3 = it.next().unwrap();
            let c4 = it.next().unwrap();

            let id = c1
                .select(&Selector::parse("a").unwrap())
                .next()
                .unwrap()
                .inner_html();

            let name = c2
                .select(&Selector::parse("a").unwrap())
                .next()
                .unwrap()
                .inner_html();

            let url = c2
                .select(&Selector::parse("a").unwrap())
                .next()
                .unwrap()
                .value()
                .attr("href")
                .unwrap();

            let tle = c3.inner_html();
            let mle = c4.inner_html();

            // dbg!(&id, &name, &url, &tle, &mle);
            problems.push(Problem {
                id: id.trim().to_owned(),
                name: name.trim().to_owned(),
                url: url.trim().to_owned(),
                tle: tle.trim().to_owned(),
                mle: mle.trim().to_owned(),
            });
        }

        Ok(ContestInfo { problems })
    }

    async fn test_cases(&self, problem_url: &str) -> Result<Vec<TestCase>> {
        let doc = http_get(
            &self.client,
            &format!("{}{}", ATCODER_ENDPOINT, problem_url),
        )
        .await?;
        // eprintln!("{}", doc);
        let doc = Html::parse_document(&doc);

        let mut inputs = vec![];
        let mut outputs = vec![];

        for r in doc.select(&Selector::parse("h3+pre").unwrap()) {
            let label = r
                .prev_sibling()
                .unwrap()
                .first_child()
                .unwrap()
                .value()
                .as_text()
                .unwrap();

            if label.starts_with("Sample Input") {
                inputs.push(r.inner_html().trim().to_owned());
            }
            if label.starts_with("Sample Output") {
                outputs.push(r.inner_html().trim().to_owned());
            }
        }

        assert_eq!(inputs.len(), outputs.len());

        let mut ret = vec![];

        for i in 0..inputs.len() {
            ret.push(TestCase {
                input: inputs[i].clone(),
                output: outputs[i].clone(),
            });
        }

        Ok(ret)
    }

    async fn submit(&self, contest_id: &str, problem_id: &str, source_code: &str) -> Result<()> {
        let doc = http_get(
            &self.client,
            &format!("{}/contests/{}/submit", ATCODER_ENDPOINT, contest_id),
        )
        .await?;
        let doc = Html::parse_document(&doc);

        let task_screen_name = (|| {
            for r in
                doc.select(&Selector::parse("select[name=\"data.TaskScreenName\"] option").unwrap())
            {
                if r.inner_html()
                    .trim()
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .to_lowercase()
                    .starts_with(&problem_id.to_lowercase())
                {
                    return Ok(r.value().attr("value").unwrap());
                }
            }
            Err(anyhow!("Problem not found: {}", problem_id))
        })()?;

        dbg!(&task_screen_name);

        let (language_id, language_name) = (|| {
            for r in doc.select(
                &Selector::parse(&format!(
                    "div[id=\"select-lang-{}\"] select option",
                    &task_screen_name
                ))
                .unwrap(),
            ) {
                if r.inner_html()
                    .trim()
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .to_lowercase()
                    .starts_with("rust")
                {
                    return Ok((r.value().attr("value").unwrap(), r.inner_html()));
                }
            }
            Err(anyhow!(
                "Rust seems to be not available in problem {}...",
                problem_id
            ))
        })()?;

        let csrf_token = doc
            .select(&Selector::parse("input[name=\"csrf_token\"]").unwrap())
            .next()
            .unwrap()
            .value()
            .attr("value")
            .unwrap();

        // dbg!(language_id, language_name, csrf_token);

        println!(
            "Submit to problem={}, using language={}",
            task_screen_name, language_name
        );

        let _res = self
            .client
            .post(&format!(
                "{}/contests/{}/submit",
                ATCODER_ENDPOINT, contest_id
            ))
            .form(&[
                ("data.TaskScreenName", task_screen_name),
                ("data.LanguageId", language_id),
                ("sourceCode", source_code),
                ("csrf_token", csrf_token),
            ])
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;

        Ok(())
    }
}

#[derive(Debug)]
struct ContestInfo {
    problems: Vec<Problem>,
}

#[derive(Debug)]
struct Problem {
    id: String,
    name: String,
    url: String,
    tle: String,
    mle: String,
}

#[derive(Debug, Clone)]
struct TestCase {
    input: String,
    output: String,
}

impl ContestInfo {
    fn problem(&self, id: &str) -> Option<&Problem> {
        self.problems
            .iter()
            .find(|p| p.id.to_lowercase() == id.to_lowercase())
    }
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
