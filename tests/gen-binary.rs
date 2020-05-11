#![cfg(target_os = "linux")]

use std::path::Path;
use std::time::Duration;
use std::{env, fs};
use tempdir::TempDir;

const TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn no_upx_no_use_cross() -> anyhow::Result<()> {
    no_upx(
        "cargo-atcoder-test-gen-binary-no-upx-no-use-cross",
        false,
        Duration::from_secs(10),
    )
}

#[test]
fn no_upx_use_cross() -> anyhow::Result<()> {
    no_upx(
        "cargo-atcoder-test-gen-binary-no-upx-use-cross",
        true,
        Duration::from_secs(300),
    )
}

fn no_upx(
    tempdir_prefix: &str,
    use_cross: bool,
    timeout_for_use_cross: Duration,
) -> anyhow::Result<()> {
    let tempdir = TempDir::new(tempdir_prefix)?;

    assert_no_manifest(tempdir.path());

    cargo_atcoder_new(tempdir.path())?;

    let config_path = tempdir.path().join("cargo-atcoder.toml");
    let mut config = fs::read_to_string(&config_path)?.parse::<toml_edit::Document>()?;
    config["atcoder"]["use_cross"] = toml_edit::value(use_cross);
    fs::write(config_path, config.to_string())?;

    fs::write(
        tempdir
            .path()
            .join("language-test-202001")
            .join("src")
            .join("bin")
            .join("practicea.rs"),
        PRACTICEA_RS,
    )?;

    assert_cmd::Command::cargo_bin("cargo-atcoder")?
        .arg("atcoder")
        .arg("gen-binary")
        .arg("--no-upx")
        .arg("--manifest-path")
        .arg(
            tempdir
                .path()
                .join("language-test-202001")
                .join("Cargo.toml"),
        )
        .arg("practicea")
        .env("CARGO_ATCODER_TEST_CONFIG_DIR", tempdir.path())
        .env("CARGO_ATCODER_TEST_CACHE_DIR", tempdir.path())
        .current_dir(tempdir.path())
        .timeout(timeout_for_use_cross)
        .assert()
        .success();

    let rustc = env::var_os("RUSTC").map(Into::into).unwrap_or_else(|| {
        let cargo = env::var_os("CARGO").unwrap();
        Path::new(&cargo).with_file_name(if cfg!(windows) { "rustc.exe" } else { "rustc" })
    });

    assert_cmd::Command::new(rustc)
        .arg("--edition")
        .arg("2018")
        .arg("-C")
        .arg("opt-level=3")
        .arg("-o")
        .arg(tempdir.path().join("practicea"))
        .arg(tempdir.path().join("practicea-bin.rs"))
        .timeout(TIMEOUT)
        .assert()
        .success()
        .stdout("")
        .stderr("");

    assert_cmd::Command::new(tempdir.path().join("practicea"))
        .write_stdin(b"1\n2 3\ntest\n"[..].to_owned())
        .timeout(TIMEOUT)
        .assert()
        .success()
        .stdout("6 test\n")
        .stderr("");
    Ok(())
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
            "-b",
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

static PRACTICEA_RS: &str = r#"use std::io::{self, Read as _};

fn main() {
    let mut input = "".to_owned();
    io::stdin().read_to_string(&mut input).unwrap();
    let mut input = input.split_ascii_whitespace();
    macro_rules! read(($ty:ty) => (input.next().unwrap().parse::<$ty>().unwrap()));

    let (a, b, c, s) = (read!(u32), read!(u32), read!(u32), read!(String));
    println!("{} {}", a + b + c, s);
}
"#;
