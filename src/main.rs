use std::{
    cmp::max,
    collections::BTreeMap,
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::{bail, ensure, Context as _, Result};
use bytesize::ByteSize;
use cargo_metadata::{Metadata, Package, Target};
use chrono::{DateTime, Local};
use console::Style;
use futures::join;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use once_cell::sync::Lazy;
use regex::Regex;
use sha2::digest::Digest;
use structopt::StructOpt;
use tokio::time::sleep;
use unicode_width::UnicodeWidthStr as _;

use crate::metadata::{MetadataExt as _, PackageExt as _};

mod atcoder;
mod config;
mod http;
mod metadata;

#[cfg(feature = "watch")]
mod watch;

use atcoder::*;
use config::{read_config, read_config_preserving, Config};

fn session_file() -> Result<PathBuf> {
    let dir = if let Some(dir) = env::var_os("CARGO_ATCODER_TEST_CACHE_DIR") {
        dir.into()
    } else {
        dirs::cache_dir()
            .with_context(|| "failed to get cache dir")?
            .join("cargo-atcoder")
    };

    if !dir.is_dir() {
        if dir.exists() {
            bail!("{} is not directory", dir.display());
        }
        fs::create_dir_all(&dir)?;
    }

    Ok(dir.join("session.txt"))
}

#[derive(StructOpt)]
struct NewOpt {
    /// Contest ID (e.g. abc123)
    contest_id: String,

    /// Create src/bin/<NAME>.rs without retrieving actual problem IDs
    #[structopt(short, long, value_name("NAME"))]
    bins: Vec<String>,

    /// Set edition
    #[structopt(long)]
    edition: Option<String>,

    /// Skip warming-up after creating project.
    #[structopt(long)]
    skip_warmup: bool,
}

async fn new_project(opt: NewOpt) -> Result<()> {
    let config = read_config()?;

    let bins = if !opt.bins.is_empty() {
        opt.bins
    } else {
        let atc = AtCoder::new(&session_file()?)?;

        match atc.contest_info(&opt.contest_id).await {
            Ok(info) => info.problem_ids_lowercase(),
            Err(err) if http::is_http_error(&err, reqwest::StatusCode::NOT_FOUND) => atc
                .problem_ids_from_score_table(&opt.contest_id)
                .await?
                .map(|ss| ss.iter().map(|s| s.to_lowercase()).collect())
                .with_context(|| {
                    err.context("could not find problem names. please specify names with `--bins`")
                })?,
            Err(err) => Err(err)?,
        }
    };

    let dir = Path::new(&opt.contest_id);
    if dir.is_dir() || dir.is_file() {
        bail!("Directory {} already exists", dir.display());
    }

    let stat = Command::new("cargo")
        .arg("new")
        .args(
            opt.edition
                .map(|edition| vec!["--edition".to_owned(), edition])
                .unwrap_or_default(),
        )
        .arg(&opt.contest_id)
        .status()?;
    if !stat.success() {
        bail!("Failed to create project: {}", &opt.contest_id);
    }

    if let Some(rustc_version) = &config.project.rustc_version {
        fs::write(dir.join("rust-toolchain"), rustc_version)?;
    }

    fs::remove_file(dir.join("src").join("main.rs"))?;
    fs::create_dir(dir.join("src").join("bin"))?;

    for bin in bins {
        fs::write(
            dir.join("src").join("bin").join(bin).with_extension("rs"),
            &config.project.template,
        )?;
    }

    let toml_file = dir.join("Cargo.toml");
    let mut manifest = fs::read_to_string(&toml_file)?.parse::<toml_edit::Document>()?;
    let conf_preserved = read_config_preserving()?;
    manifest["dependencies"] = conf_preserved["dependencies"].clone();
    manifest["dev-dependencies"] = conf_preserved["dev-dependencies"].clone();
    manifest["profile"] = toml_edit::Item::Table({
        let mut tbl = toml_edit::Table::new();
        tbl.set_implicit(true);
        tbl
    });
    manifest["profile"]["release"] = conf_preserved["profile"]["release"].clone();

    fs::write(toml_file, manifest.to_string())?;

    println!("Creating project done.");

    if !opt.skip_warmup {
        let metadata = metadata::cargo_metadata(None, format!("./{}", opt.contest_id).as_ref())?;
        warmup_for(&metadata, Some(&[&opt.contest_id]))?;
        println!("Warming up done.");
    }

    Ok(())
}

async fn login() -> Result<()> {
    let username = dialoguer::Input::<String>::new()
        .with_prompt("Username")
        .interact()?;

    let password = dialoguer::Password::new()
        .with_prompt("Password")
        .interact()?;

    let atc = AtCoder::new(&session_file()?)?;
    atc.login(&username, &password).await?;

    println!("Login succeeded.");

    Ok(())
}

fn clear_session() -> Result<()> {
    let path = session_file()?;
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
    /// [cargo] Package with the target to test
    #[structopt(short, long, value_name("SPEC"))]
    package: Option<String>,
    /// [cargo] Path to Cargo.toml
    #[structopt(long, value_name("PATH"))]
    manifest_path: Option<PathBuf>,
    /// Use custom case from stdin
    #[structopt(short, long, conflicts_with = "case_num")]
    custom: bool,
    /// Submit if test passed
    #[structopt(short, long)]
    submit: bool,
    /// [cargo build] Use --release flag to compile
    #[structopt(long)]
    release: bool,
    /// Use verbose output
    #[structopt(short, long)]
    verbose: bool,
}

async fn test(opt: TestOpt) -> Result<()> {
    let cwd = env::current_dir().with_context(|| "failed to get CWD")?;
    let metadata = metadata::cargo_metadata(opt.manifest_path.as_deref(), &cwd)?;
    let package = metadata.query_for_member(opt.package.as_deref())?;
    let atc = AtCoder::new(&session_file()?)?;
    let problem_id = opt.problem_id;
    let contest_id = &package.name;
    let contest_info = atc.contest_info(contest_id).await?;

    let problem = contest_info
        .problem(&problem_id)
        .with_context(|| format!("Problem `{}` is not contained in this contest", &problem_id))?;

    if opt.custom {
        return test_custom(package, &problem_id, opt.release);
    }

    let test_cases = atc.test_cases(&problem.url).await?;

    for &cn in opt.case_num.iter() {
        if cn == 0 || cn > test_cases.len() {
            bail!(
                "Case num {} is not found in problem {} samples",
                cn,
                problem_id
            );
        }
    }

    let mut tcs = vec![];
    for (i, tc) in test_cases.into_iter().enumerate() {
        if opt.case_num.is_empty() || opt.case_num.contains(&(i + 1)) {
            tcs.push((i, tc));
        }
    }

    let passed = test_samples(package, &problem_id, &tcs, opt.release, opt.verbose)?;
    if passed && opt.submit {
        let Target { src_path, .. } = package.find_bin(&problem_id)?;
        let source = fs::read(src_path).with_context(|| format!("Failed to read {}", src_path))?;
        atc.submit(contest_id, &problem_id, &String::from_utf8_lossy(&source))
            .await?;
    }

    Ok(())
}

fn test_samples(
    package: &Package,
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
        .arg("--manifest-path")
        .arg(&package.manifest_path)
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
            .arg("--manifest-path")
            .arg(&package.manifest_path)
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

        let cmp_res = cmp_output(&stdout, &test_case.output);
        let ferr = if let Some(ferr) = cmp_res.1 {
            format!(
                " (abs error: {:<10.3e}, rel error: {:<10.3e})",
                ferr.abs_error, ferr.rel_error
            )
        } else {
            "".to_string()
        };

        if !cmp_res.0 {
            println!(
                "test sample {} ... {}{}",
                i + 1,
                red.apply_to("FAILED"),
                ferr
            );
            fails.push((i, true, output));
        } else {
            println!("test sample {} ... {}{}", i + 1, green.apply_to("ok"), ferr);
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

const ERROR_THRESHOLD: f64 = 1e-6;

#[derive(Debug)]
struct FloatError {
    abs_error: f64,
    rel_error: f64,
}

// returns (accepted?, maximum float error if float value exists)
fn cmp_output(reference: &str, out: &str) -> (bool, Option<FloatError>) {
    let mut max_error = None;

    let ws1 = reference.split_whitespace().collect::<Vec<_>>();
    let ws2 = out.split_whitespace().collect::<Vec<_>>();

    if ws1.len() != ws2.len() {
        return (false, None);
    }

    for i in 0..ws1.len() {
        let w1 = ws1[i];
        let w2 = ws2[i];

        if (is_float(w1) || is_float(w2))
            && (is_float(w1) || is_integer(w1))
            && (is_float(w2) || is_integer(w2))
        {
            let f1 = w1.parse::<f64>().unwrap();
            let f2 = w2.parse::<f64>().unwrap();

            let abs_error = (f1 - f2).abs();
            let rel_error = abs_error / f1.abs();

            if max_error.is_none() {
                max_error = Some(FloatError {
                    abs_error: 0.,
                    rel_error: 0.,
                });
            }

            max_error = Some({
                FloatError {
                    abs_error: abs_error.max(max_error.as_ref().unwrap().abs_error),
                    rel_error: rel_error.max(max_error.as_ref().unwrap().rel_error),
                }
            });
        } else if w1 != w2 {
            return (false, None);
        }
    }

    if let Some(max_error) = max_error {
        let ok = max_error.abs_error.min(max_error.rel_error) < ERROR_THRESHOLD;
        return (ok, Some(max_error));
    }

    (true, max_error)
}

static FLOAT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d+\.\d+$").unwrap());
static INTEGER_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\d+$").unwrap());

fn is_float(w: &str) -> bool {
    FLOAT_RE.is_match(w)
}

fn is_integer(w: &str) -> bool {
    INTEGER_RE.is_match(w)
}

fn test_custom(package: &Package, problem_id: &str, release: bool) -> Result<()> {
    let build_status = Command::new("cargo")
        .arg("build")
        .args(if release { vec!["--release"] } else { vec![] })
        .arg("--bin")
        .arg(&problem_id)
        .arg("--manifest-path")
        .arg(&package.manifest_path)
        .status()?;

    ensure!(build_status.success(), "Build failed");

    println!("input test case:");

    let red = Style::new().red();
    let cyan = Style::new().cyan();

    let child = Command::new("cargo")
        .arg("run")
        .args(if release { vec!["--release"] } else { vec![] })
        .arg("-q")
        .arg("--bin")
        .arg(&problem_id)
        .arg("--manifest-path")
        .arg(&package.manifest_path)
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
    /// Problem ID (must be same as binary name)
    problem_id: String,
    /// [cargo] Package with the target to submit
    #[structopt(short, long, value_name("SPEC"))]
    package: Option<String>,
    /// [cargo] Path to Cargo.toml
    #[structopt(long, value_name("PATH"))]
    manifest_path: Option<PathBuf>,
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
    /// Max column number of generated binary
    column: Option<usize>,
    /// Do no use upx unless available
    #[structopt(long)]
    no_upx: bool,
    /// [cargo build] Use --release on pre-test (submission always uses --release)
    #[structopt(long)]
    release: bool,
}

async fn submit(opt: SubmitOpt) -> Result<()> {
    let cwd = env::current_dir().with_context(|| "failed to get CWD")?;
    let metadata = metadata::cargo_metadata(opt.manifest_path.as_deref(), &cwd)?;
    let package = metadata.query_for_member(opt.package.as_deref())?;
    let atc = AtCoder::new(&session_file()?)?;
    let config = read_config()?;

    let contest_id = &package.name;
    let problem_id = opt.problem_id;
    let contest_info = atc.contest_info(contest_id).await?;
    let problem = contest_info
        .problem(&problem_id)
        .with_context(|| format!("Problem `{}` is not contained in this contest", &problem_id))?;

    let test_passed = if opt.skip_test {
        true
    } else {
        let test_cases = atc
            .test_cases(&problem.url)
            .await?
            .into_iter()
            .enumerate()
            .collect::<Vec<_>>();
        test_samples(package, &problem_id, &test_cases, opt.release, false)?
    };

    if !test_passed && !opt.force {
        println!("Sample test failed. Did not submit.");
        return Ok(());
    }

    let via_bin = opt.bin || (config.atcoder.submit_via_binary && !opt.source);
    let target = package.find_bin(&problem_id)?;
    let source = if !via_bin {
        let Target { src_path, .. } = target;
        fs::read(src_path).with_context(|| format!("Failed to read {}", src_path))?
    } else {
        println!("Submitting via binary...");
        gen_binary_source(&metadata, package, &target, &config, opt.column, opt.no_upx)?
    };

    atc.submit(contest_id, &problem_id, &String::from_utf8_lossy(&source))
        .await?;
    println!();

    println!("Fetching submission result...");
    let atc = Arc::new(atc);
    let last_id = watch_submission_status(Arc::clone(&atc), contest_id, true).await?;
    println!();

    if let Some(last_id) = last_id {
        let res = atc.submission_status_full(contest_id, last_id).await?;
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

fn gen_binary_source(
    metadata: &Metadata,
    package: &Package,
    bin: &Target,
    config: &Config,
    column: Option<usize>,
    no_upx: bool,
) -> Result<Vec<u8>> {
    let source_code = fs::read_to_string(&bin.src_path)
        .with_context(|| format!("Failed to read {}", bin.src_path))?;

    let target = &config.profile.target;
    let binary_file = metadata
        .target_directory
        .join(target)
        .join("release")
        .join(&bin.name);

    let program = if config.atcoder.use_cross {
        "cross"
    } else {
        "cargo"
    };

    if which::which(program).is_err() {
        bail!("Build failed. {} not found.", program);
    }

    let status = Command::new(program)
        .arg("build")
        .arg(format!("--target={}", target))
        .arg("--release")
        .arg("--bin")
        .arg(&bin.name)
        .current_dir({
            // `cross` does not work with `--manifest-path <absolute path>`.
            package
                .manifest_path
                .parent()
                .expect("`manifest_path` should end with \"Cargo.toml\"")
        })
        .status()?;

    ensure!(status.success(), "Build failed");

    let size = ByteSize::b(get_file_size(&binary_file)?);
    println!("Built binary size: {}", size);

    let status = Command::new(match config.atcoder.strip_path {
        Some(ref p) => p,
        None => "strip",
    })
    .arg("-s")
    .arg(&binary_file)
    .status()?;
    ensure!(status.success(), "strip failed");

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
            ensure!(status.success(), "upx failed");
            let size = ByteSize::b(get_file_size(&binary_file)?);
            println!("Compressed binary size: {}", size);
        }
    } else if !no_upx {
        println!("upx not found. Binary is not compressed.");
    }

    let code = {
        let templ = include_str!("../data/binary_runner.rs.txt");

        let bin = fs::read(&binary_file)?;

        let column = column.unwrap_or(config.atcoder.binary_column);
        let bin_base64 = data_encoding::BASE64.encode(&bin);
        let bin_base64 = if column > 0 {
            split_lines(&bin_base64, column)
        } else {
            bin_base64
        };

        let code = templ.replace("{{SOURCE_CODE}}", source_code.trim_end());
        let code = code.replace(
            "{{HASH}}",
            &data_encoding::HEXUPPER.encode(&sha2::Sha256::digest(&bin))[0..8],
        );
        code.replace("{{BINARY}}", &bin_base64)
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

fn split_lines(s: &str, w: usize) -> String {
    let mut s = s;

    let mut ret = String::new();
    while s.len() > w {
        let (a, b) = s.split_at(w);
        ret += a;
        ret.push('\n');
        s = b;
    }

    if !s.is_empty() {
        ret += s;
        ret.push('\n');
    }

    ret
}

async fn info() -> Result<()> {
    let atc = AtCoder::new(&session_file()?)?;

    if let Some(username) = atc.username().await? {
        println!("Logged in as {}.", username);
    } else {
        println!("Not logged in.");
    }

    Ok(())
}

#[derive(StructOpt, Debug)]
struct WarmupOpt {
    /// [cargo] Package(s) to warm up
    #[structopt(short, long, value_name("SPEC"), min_values(1), number_of_values(1))]
    package: Vec<String>,

    /// [cargo] Path to Cargo.toml
    #[structopt(long, value_name("PATH"))]
    manifest_path: Option<PathBuf>,
}

fn warmup(opt: WarmupOpt) -> Result<()> {
    let cwd = env::current_dir().with_context(|| "failed to get CWD")?;
    let metadata = metadata::cargo_metadata(opt.manifest_path.as_deref(), &cwd)?;
    warmup_for(&metadata, Some(&*opt.package).filter(|ss| !ss.is_empty()))
}

fn warmup_for(metadata: &Metadata, specs: Option<&[impl AsRef<str>]>) -> Result<()> {
    let members = specs
        .map(|specs| {
            specs
                .iter()
                .map(|s| metadata.query_for_member(Some(s.as_ref())))
                .collect()
        })
        .unwrap_or_else(|| Ok(metadata.all_members()))?;

    for member in members {
        if let Some(first_bin) = member.all_bins().get(0) {
            println!("Warming up debug build for `{}`...", member.name);

            let stat = Command::new("cargo")
                .arg("build")
                .arg("--manifest-path")
                .arg(&member.manifest_path)
                .arg("--bin")
                .arg(&first_bin.name)
                .status()?;

            if !stat.success() {
                eprintln!("Failed to warm-up");
            }

            println!("Warming up release build for `{}`...", member.name);

            let stat = Command::new("cargo")
                .arg("build")
                .arg("--manifest-path")
                .arg(&member.manifest_path)
                .arg("--release")
                .arg("--bin")
                .arg(&first_bin.name)
                .status()?;

            if !stat.success() {
                eprintln!("Failed to warm-up");
            }
        }
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

                    let problem_name_head = {
                        let mut problem_name_head = result.problem_name.clone();
                        // TODO: `result.problem_name` possibly contains ambiguous width characters such as emoji.
                        while problem_name_head.width() > 20 {
                            problem_name_head.pop();
                        }
                        for _ in 0..20usize.saturating_sub(problem_name_head.width()) {
                            problem_name_head.push(' ');
                        }
                        problem_name_head
                    };

                    pb.set_prefix(format!(
                        "{} | {} |",
                        DateTime::<Local>::from(result.date).format("%Y-%m-%d %H:%M:%S"),
                        problem_name_head,
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
                            pb.0.set_message(format!(
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
                            pb.0.finish_with_message(format!(
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
                sleep(Duration::from_millis(refresh_rate)).await;
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
    /// [cargo] Path to Cargo.toml
    #[structopt(long, value_name("PATH"))]
    manifest_path: Option<PathBuf>,
    /// Output filename (default: <problem-id>-bin.rs)
    #[structopt(long, short)]
    output: Option<PathBuf>,
    /// Max column number of generated binary
    #[structopt(long, short)]
    column: Option<usize>,
    /// Do not use UPX even if it is available
    #[structopt(long)]
    no_upx: bool,
}

fn gen_binary(opt: GenBinaryOpt) -> Result<()> {
    let cwd = env::current_dir().with_context(|| "failed to get CWD")?;
    let metadata = metadata::cargo_metadata(opt.manifest_path.as_deref(), &cwd)?;
    let (target, package) = metadata.find_bin(&opt.problem_id)?;
    let config = read_config()?;
    let src = gen_binary_source(&metadata, package, target, &config, opt.column, opt.no_upx)?;
    let filename = opt
        .output
        .clone()
        .unwrap_or_else(|| PathBuf::from(format!("{}-bin.rs", opt.problem_id)));
    fs::write(&filename, &src)?;
    println!("Wrote code to `{}`", filename.display());
    Ok(())
}

#[derive(StructOpt)]
struct ResultOpt {
    /// submission ID
    submission_id: usize,
    /// [cargo] Package
    #[structopt(short, long, value_name("SPEC"))]
    package: Option<String>,
    /// [cargo] Path to Cargo.toml
    #[structopt(long, value_name("PATH"))]
    manifest_path: Option<PathBuf>,
    /// Use verbose output
    #[structopt(long, short)]
    verbose: bool,
}

async fn result(opt: ResultOpt) -> Result<()> {
    let cwd = env::current_dir().with_context(|| "failed to get CWD")?;
    let metadata = metadata::cargo_metadata(opt.manifest_path.as_deref(), &cwd)?;
    let atc = AtCoder::new(&session_file()?)?;
    let contest_id = &metadata.query_for_member(opt.package.as_deref())?.name;
    let res = atc
        .submission_status_full(contest_id, opt.submission_id)
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
        res.result.run_time.as_deref().unwrap_or("N/A")
    );
    println!(
        "Memory:        {}",
        res.result.memory.as_deref().unwrap_or("N/A")
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
                    case.run_time.clone().unwrap_or_else(|| "N/A".to_string()),
                    case.memory.clone().unwrap_or_else(|| "N/A".to_string())
                );
            }
        }
    }
    Ok(())
}

#[derive(StructOpt, Debug)]
struct StatusOpt {
    /// [cargo] Package
    #[structopt(short, long, value_name("SPEC"))]
    package: Option<String>,
    /// [cargo] Path to Cargo.toml
    #[structopt(long, value_name("PATH"))]
    manifest_path: Option<PathBuf>,
}

async fn status(opt: StatusOpt) -> Result<()> {
    let cwd = env::current_dir().with_context(|| "failed to get CWD")?;
    let metadata = metadata::cargo_metadata(opt.manifest_path.as_deref(), &cwd)?;
    let atc = AtCoder::new(&session_file()?)?;
    let contest_id = &metadata.query_for_member(opt.package.as_deref())?.name;
    let atc = Arc::new(atc);
    watch_submission_status(atc, contest_id, false).await?;
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
    /// Warmup (pre-compile dependencies)
    Warmup(WarmupOpt),
    /// Test sample cases
    Test(TestOpt),
    /// Submit solution
    Submit(SubmitOpt),
    /// Show submission result detail
    Result(ResultOpt),
    /// Generate rustified binary
    GenBinary(GenBinaryOpt),
    /// Show submission status
    Status(StatusOpt),

    /// [WIP] Watch filesystem for automatic submission
    #[cfg(feature = "watch")]
    Watch(watch::WatchOpt),
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
        Warmup(opt) => warmup(opt),
        Test(opt) => test(opt).await,
        Submit(opt) => submit(opt).await,
        Result(opt) => result(opt).await,
        GenBinary(opt) => gen_binary(opt),
        Status(opt) => status(opt).await,

        #[cfg(feature = "watch")]
        Watch(opt) => watch::watch(opt).await,
    }
}
