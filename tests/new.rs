use cargo_metadata::{Metadata, MetadataCommand, Package};
use maplit::btreemap;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempdir::TempDir;

const TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn default() -> anyhow::Result<()> {
    let tempdir = TempDir::new("cargo-atcoder-test-new-default")?;

    assert_no_manifest(tempdir.path());

    assert_cmd::Command::cargo_bin("cargo-atcoder")?
        .args(&["atcoder", "new", "abc126"])
        .env("CARGO_ATCODER_TEST_CONFIG_DIR", tempdir.path())
        .env("CARGO_ATCODER_TEST_CACHE_DIR", tempdir.path())
        .current_dir(tempdir.path())
        .timeout(TIMEOUT)
        .assert()
        .success();

    let metadata = cargo_metadata(&tempdir.path().join("abc126").join("Cargo.toml"), true)?;

    assert_eq!(tempdir.path().join("abc126"), metadata.workspace_root);
    assert_is_git_root(metadata.workspace_root.as_ref());
    assert_build_cache_exists(metadata.workspace_root.as_ref());
    assert_bin_names(
        find_member(&metadata, "abc126"),
        &btreemap!(
            "a" => Path::new("src").join("bin").join("a.rs"),
            "b" => Path::new("src").join("bin").join("b.rs"),
            "c" => Path::new("src").join("bin").join("c.rs"),
            "d" => Path::new("src").join("bin").join("d.rs"),
            "e" => Path::new("src").join("bin").join("e.rs"),
            "f" => Path::new("src").join("bin").join("f.rs"),
        ),
    );

    tempdir.close().map_err(Into::into)
}

#[test]
fn edition() -> anyhow::Result<()> {
    let tempdir = TempDir::new("cargo-atcoder-test-new-edition")?;

    assert_no_manifest(tempdir.path());

    assert_cmd::Command::cargo_bin("cargo-atcoder")?
        .args(&["atcoder", "new", "--edition", "2018", "abc126"])
        .env("CARGO_ATCODER_TEST_CONFIG_DIR", tempdir.path())
        .env("CARGO_ATCODER_TEST_CACHE_DIR", tempdir.path())
        .current_dir(tempdir.path())
        .timeout(TIMEOUT)
        .assert()
        .success();

    let metadata = cargo_metadata(&tempdir.path().join("abc126").join("Cargo.toml"), false)?;

    assert_eq!(find_member(&metadata, "abc126").edition, "2018");

    tempdir.close().map_err(Into::into)
}

#[test]
fn skip_warmup() -> anyhow::Result<()> {
    let tempdir = TempDir::new("cargo-atcoder-test-new-skip-warmup")?;

    assert_no_manifest(tempdir.path());

    assert_cmd::Command::cargo_bin("cargo-atcoder")?
        .args(&["atcoder", "new", "--skip-warmup", "abc126"])
        .env("CARGO_ATCODER_TEST_CONFIG_DIR", tempdir.path())
        .env("CARGO_ATCODER_TEST_CACHE_DIR", tempdir.path())
        .current_dir(tempdir.path())
        .timeout(TIMEOUT)
        .assert()
        .success();

    let metadata = cargo_metadata(&tempdir.path().join("abc126").join("Cargo.toml"), false)?;

    assert_eq!(tempdir.path().join("abc126"), metadata.workspace_root);
    assert_is_git_root(metadata.workspace_root.as_ref());
    assert_build_cache_not_exist(metadata.workspace_root.as_ref());
    assert_bin_names(
        find_member(&metadata, "abc126"),
        &btreemap!(
            "a" => Path::new("src").join("bin").join("a.rs"),
            "b" => Path::new("src").join("bin").join("b.rs"),
            "c" => Path::new("src").join("bin").join("c.rs"),
            "d" => Path::new("src").join("bin").join("d.rs"),
            "e" => Path::new("src").join("bin").join("e.rs"),
            "f" => Path::new("src").join("bin").join("f.rs"),
        ),
    );

    tempdir.close().map_err(Into::into)
}

#[test]
fn bins() -> anyhow::Result<()> {
    let tempdir = TempDir::new("cargo-atcoder-test-new-bins")?;

    assert_no_manifest(tempdir.path());

    assert_cmd::Command::cargo_bin("cargo-atcoder")?
        .args(&[
            "atcoder", "new", "abc999", "--bins", "v", "w", "x", "y", "z",
        ])
        .env("CARGO_ATCODER_TEST_CONFIG_DIR", tempdir.path())
        .env("CARGO_ATCODER_TEST_CACHE_DIR", tempdir.path())
        .current_dir(tempdir.path())
        .timeout(TIMEOUT)
        .assert()
        .success();

    let metadata = cargo_metadata(&tempdir.path().join("abc999").join("Cargo.toml"), true)?;

    assert_eq!(tempdir.path().join("abc999"), metadata.workspace_root);
    assert_is_git_root(metadata.workspace_root.as_ref());
    assert_build_cache_exists(metadata.workspace_root.as_ref());
    assert_bin_names(
        find_member(&metadata, "abc999"),
        &btreemap!(
            "v" => Path::new("src").join("bin").join("v.rs"),
            "w" => Path::new("src").join("bin").join("w.rs"),
            "x" => Path::new("src").join("bin").join("x.rs"),
            "y" => Path::new("src").join("bin").join("y.rs"),
            "z" => Path::new("src").join("bin").join("z.rs"),
        ),
    );

    tempdir.close().map_err(Into::into)
}

fn assert_no_manifest(dir: &Path) {
    if let Some(manifest_dir) = dir.ancestors().find(|p| p.join("Cargo.toml").exists()) {
        panic!("found Cargo.toml at {}", manifest_dir.display());
    }
}

fn cargo_metadata(manifest_path: &Path, frozen: bool) -> cargo_metadata::Result<Metadata> {
    let mut cmd = MetadataCommand::new();
    if frozen {
        cmd.other_options(vec!["--frozen".to_owned()]);
    }
    cmd.manifest_path(manifest_path).exec()
}

fn find_member<'a>(metadata: &'a Metadata, name: &str) -> &'a Package {
    metadata
        .packages
        .iter()
        .find(|p| metadata.workspace_members.contains(&p.id) && p.name == name)
        .unwrap_or_else(|| panic!("{}: `{}` not found", metadata.workspace_root, name,))
}

fn assert_is_git_root(workspace_root: &Path) {
    assert!(workspace_root.join(".git").is_dir());
}

fn assert_build_cache_exists(workspace_root: &Path) {
    assert!(workspace_root.join("target").join("debug").is_dir());
    assert!(workspace_root.join("target").join("release").is_dir());
}

fn assert_build_cache_not_exist(workspace_root: &Path) {
    assert!(!workspace_root.join("target").exists());
}

fn assert_bin_names(package: &Package, bins: &BTreeMap<&str, PathBuf>) {
    let mut actual_bins = BTreeMap::<&str, PathBuf>::new();
    for target in &package.targets {
        assert_eq!(vec!["bin"], target.kind);
        let path = target
            .src_path
            .strip_prefix(package.manifest_path.parent().unwrap())
            .unwrap()
            .to_owned();
        actual_bins.insert(&*target.name, {
            let path: &Path = path.as_ref();
            path.to_owned()
        });
    }
    assert_eq!(*bins, actual_bins);
}
