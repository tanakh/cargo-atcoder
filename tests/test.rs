use assert_cmd::assert::Assert;
use predicates::Predicate;
use std::path::Path;
use std::time::Duration;
use std::{fs, str};
use tempdir::TempDir;

const TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn samples_for_existing_ws() -> anyhow::Result<()> {
    samples("cargo-atcoder-test-test-samples-for-existing-ws", true)
}

#[test]
fn samples_for_missing_ws() -> anyhow::Result<()> {
    samples("cargo-atcoder-test-test-samples-for-missing-ws", false)
}

#[test]
fn custom_input_for_existing_ws() -> anyhow::Result<()> {
    custom_input("cargo-atcoder-test-test-custom_input-for-existing-ws", true)
}

#[test]
fn custom_input_for_missing_ws() -> anyhow::Result<()> {
    custom_input("cargo-atcoder-test-test-custom_input-for-missing-ws", false)
}

fn samples(tempdir_prefix: &str, create_virtual_manifest: bool) -> anyhow::Result<()> {
    let tempdir = TempDir::new(tempdir_prefix)?;

    assert_no_manifest(tempdir.path());

    if create_virtual_manifest {
        fs::write(tempdir.path().join("Cargo.toml"), "[workspace]\n")?;
    }

    cargo_atcoder_new(tempdir.path())?;

    let run =
        |code, status, stdout, stderr| run(tempdir.path(), code, None, status, stdout, stderr);

    run(
        AC,
        Assert::success,
        |stdout| {
            stdout
                == r#"running 2 tests
test sample 1 ... ok
test sample 2 ... ok

test_result: ok

"#
        },
        |stderr| stderr.contains("   Compiling language-test-202001 v0.1.0"),
    )?;

    run(
        RE,
        Assert::success,
        |stdout| {
            stdout.contains(
                r#"running 2 tests
test sample 1 ... FAILED
test sample 2 ... FAILED
"#,
            ) && stdout.ends_with(
                r#"test result: FAILED. 0 passed; 2 failed

"#,
            )
        },
        |stderr| stderr.contains("   Compiling language-test-202001 v0.1.0"),
    )?;

    run(CE, Assert::success, str::is_empty, |stderr| {
        stderr.contains("   Compiling language-test-202001 v0.1.0")
            && stderr.contains("could not compile `language-test-202001`.\n")
    })
}

fn custom_input(tempdir_prefix: &str, create_virtual_manifest: bool) -> anyhow::Result<()> {
    let tempdir = TempDir::new(tempdir_prefix)?;

    assert_no_manifest(tempdir.path());

    if create_virtual_manifest {
        fs::write(tempdir.path().join("Cargo.toml"), "[workspace]\n")?;
    }

    cargo_atcoder_new(tempdir.path())?;

    let run = |code, custom_input, status, stdout, stderr| -> _ {
        run(
            tempdir.path(),
            code,
            Some(custom_input),
            status,
            stdout,
            stderr,
        )
    };

    run(
        AC,
        r#"1
1 1
(´･_･`)
"#,
        Assert::success,
        |stdout| {
            stdout.contains(
                r#"your output:
     1 | 3 (´･_･`)
"#,
            )
        },
        |stderr| stderr.contains("   Compiling language-test-202001 v0.1.0"),
    )?;

    run(
        AC,
        "ミ゙",
        Assert::success,
        |stdout| stdout.contains("runtime error"),
        |stderr| stderr.contains("   Compiling language-test-202001 v0.1.0"),
    )?;

    run(
        RE,
        "",
        Assert::success,
        |stdout| stdout.contains("runtime error"),
        |stderr| stderr.contains("   Compiling language-test-202001 v0.1.0"),
    )?;

    run(CE, "", Assert::failure, str::is_empty, |stderr| {
        stderr.contains("   Compiling language-test-202001 v0.1.0")
            && stderr.contains("could not compile `language-test-202001`.\n")
    })
}

fn assert_no_manifest(dir: &Path) {
    if let Some(manifest_dir) = dir.ancestors().find(|p| p.join("Cargo.toml").exists()) {
        panic!("found Cargo.toml at {}", manifest_dir.display());
    }
}

fn cargo_atcoder_new(dir: &Path) -> anyhow::Result<()> {
    assert_cmd::Command::cargo_bin("cargo-atcoder")?
        .args(&[
            "atcoder",
            "new",
            "language-test-202001",
            "--skip-warmup",
            "--problems",
            "practicea",
        ])
        .env("CARGO_ATCODER_TEST_CONFIG_DIR", dir)
        .env("CARGO_ATCODER_TEST_CACHE_DIR", dir)
        .current_dir(dir)
        .timeout(TIMEOUT)
        .assert()
        .success();
    Ok(())
}

fn run(
    dir: &Path,
    code: &str,
    custom_input: Option<&str>,
    status: fn(Assert) -> Assert,
    stdout: fn(&str) -> bool,
    stderr: fn(&str) -> bool,
) -> anyhow::Result<()> {
    let src_path = dir
        .join("language-test-202001")
        .join("src")
        .join("bin")
        .join("practicea.rs");
    fs::write(src_path, code)?;

    let assert = assert_cmd::Command::cargo_bin("cargo-atcoder")?
        .arg("atcoder")
        .arg("test")
        .args(Some("-c").filter(|_| custom_input.is_some()))
        .arg("--manifest-path")
        .arg(dir.join("language-test-202001").join("Cargo.toml"))
        .arg("practicea")
        .env("CARGO_ATCODER_TEST_CONFIG_DIR", dir)
        .env("CARGO_ATCODER_TEST_CACHE_DIR", dir)
        .write_stdin(custom_input.map(ToOwned::to_owned).unwrap_or_default())
        .current_dir(dir)
        .timeout(TIMEOUT)
        .assert();

    status(assert)
        .stdout(predicate(stdout))
        .stderr(predicate(stderr));

    return Ok(());

    fn predicate(f: fn(&str) -> bool) -> impl Predicate<[u8]> {
        predicates::function::function(move |s| str::from_utf8(s).map_or(false, f))
    }
}

static AC: &str = r#"use std::io::{self, Read as _};

fn main() {
    let mut input = "".to_owned();
    io::stdin().read_to_string(&mut input).unwrap();
    let mut input = input.split_ascii_whitespace();
    macro_rules! read(($ty:ty) => (input.next().unwrap().parse::<$ty>().unwrap()));

    let (a, b, c, s) = (read!(u32), read!(u32), read!(u32), read!(String));
    println!("{} {}", a + b + c, s);
}
"#;

static RE: &str = r#"fn main() {
    panic!();
}
"#;

static CE: &str = "ミ゙";
