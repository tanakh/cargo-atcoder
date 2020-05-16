use cargo_metadata::{Metadata, MetadataCommand, Package};
use maplit::btreemap;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use std::{fs, str};
use tempdir::TempDir;

const TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn skip_warmup_for_existing_ws() -> anyhow::Result<()> {
    let tempdir = TempDir::new("cargo-atcoder-test-new-skip-warmup-for-existing-ws")?;

    assert_no_manifest(tempdir.path());
    fs::write(tempdir.path().join("Cargo.toml"), "[workspace]\n")?;

    let dir_canonicalized = dunce::canonicalize(tempdir.path())?;
    cargo_atcoder_new(
        &dir_canonicalized,
        &["abc126", "--skip-warmup"],
        &r#"
       Added "abc126" to `workspace.members` at {{root-manifest-path}}
     Created binary (application) `abc126` package
     Removed the `main.rs` in `abc126`
       Added 6 `bin`(s) to `abc126`
    Modified `abc126` successfully
    Skipping warming up
"#[1..]
            .replace(
                "{{root-manifest-path}}",
                &dir_canonicalized.join("Cargo.toml").to_string_lossy(),
            ),
    )?;

    let metadata = cargo_metadata(&tempdir.path().join("abc126").join("Cargo.toml"))?;
    let abc126 = find_member(&metadata, "abc126");

    assert_eq!(tempdir.path(), metadata.workspace_root);
    assert_eq!(
        tempdir.path().join("abc126").join("Cargo.toml"),
        abc126.manifest_path,
    );
    assert_is_git_root(abc126.manifest_path.parent().unwrap());
    assert_build_cache_not_exist(&metadata.workspace_root);
    assert_bin_names(
        abc126,
        &btreemap!(
            "abc126-a" => Path::new("src").join("bin").join("a.rs"),
            "abc126-b" => Path::new("src").join("bin").join("b.rs"),
            "abc126-c" => Path::new("src").join("bin").join("c.rs"),
            "abc126-d" => Path::new("src").join("bin").join("d.rs"),
            "abc126-e" => Path::new("src").join("bin").join("e.rs"),
            "abc126-f" => Path::new("src").join("bin").join("f.rs"),
        ),
    );

    tempdir.close().map_err(Into::into)
}

#[test]
fn skip_warmup_bins_for_existing_ws() -> anyhow::Result<()> {
    let tempdir = TempDir::new("cargo-atcoder-test-new-skip-warmup-bins-for-existing-ws")?;

    assert_no_manifest(tempdir.path());
    fs::write(tempdir.path().join("Cargo.toml"), "[workspace]\n")?;

    let dir_canonicalized = dunce::canonicalize(tempdir.path())?;
    cargo_atcoder_new(
        &dir_canonicalized,
        &[
            "abc999",
            "--skip-warmup",
            "--problems",
            "v",
            "w",
            "x",
            "y",
            "z",
        ],
        &r#"
       Added "abc999" to `workspace.members` at {{root-manifest-path}}
     Created binary (application) `abc999` package
     Removed the `main.rs` in `abc999`
       Added 5 `bin`(s) to `abc999`
    Modified `abc999` successfully
    Skipping warming up
"#[1..]
            .replace(
                "{{root-manifest-path}}",
                &dir_canonicalized.join("Cargo.toml").to_string_lossy(),
            ),
    )?;

    let metadata = cargo_metadata(&tempdir.path().join("abc999").join("Cargo.toml"))?;
    let abc999 = find_member(&metadata, "abc999");

    assert_eq!(tempdir.path(), metadata.workspace_root);
    assert_eq!(
        tempdir.path().join("abc999").join("Cargo.toml"),
        abc999.manifest_path,
    );
    assert_is_git_root(abc999.manifest_path.parent().unwrap());
    assert_build_cache_not_exist(&metadata.workspace_root);
    assert_bin_names(
        abc999,
        &btreemap!(
            "abc999-v" => Path::new("src").join("bin").join("v.rs"),
            "abc999-w" => Path::new("src").join("bin").join("w.rs"),
            "abc999-x" => Path::new("src").join("bin").join("x.rs"),
            "abc999-y" => Path::new("src").join("bin").join("y.rs"),
            "abc999-z" => Path::new("src").join("bin").join("z.rs"),
        ),
    );

    tempdir.close().map_err(Into::into)
}

#[test]
fn skip_warmup_for_missing_ws() -> anyhow::Result<()> {
    let tempdir = TempDir::new("cargo-atcoder-test-new-skip-warmup-for-missing-ws")?;

    assert_no_manifest(tempdir.path());

    cargo_atcoder_new(
        tempdir.path(),
        &["abc126", "--skip-warmup"],
        &r#"
warning: No existing workspace found. We recommend that you manage packages in one workspace to reduce build time and disk usage. Run `cargo atcoder migrate` to unify existing workspaces. For further information about workspaces, see https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html
     Created binary (application) `abc126` package
     Removed the `main.rs` in `abc126`
       Added 6 `bin`(s) to `abc126`
    Modified `abc126` successfully
    Skipping warming up
"#[1..],
    )?;

    let metadata = cargo_metadata(&tempdir.path().join("abc126").join("Cargo.toml"))?;

    assert_eq!(tempdir.path().join("abc126"), metadata.workspace_root);
    assert_is_git_root(&metadata.workspace_root);
    assert_build_cache_not_exist(&metadata.workspace_root);
    assert_bin_names(
        find_member(&metadata, "abc126"),
        &btreemap!(
            "abc126-a" => Path::new("src").join("bin").join("a.rs"),
            "abc126-b" => Path::new("src").join("bin").join("b.rs"),
            "abc126-c" => Path::new("src").join("bin").join("c.rs"),
            "abc126-d" => Path::new("src").join("bin").join("d.rs"),
            "abc126-e" => Path::new("src").join("bin").join("e.rs"),
            "abc126-f" => Path::new("src").join("bin").join("f.rs"),
        ),
    );

    tempdir.close().map_err(Into::into)
}

#[test]
fn skip_warmup_bins_for_missing_ws() -> anyhow::Result<()> {
    let tempdir = TempDir::new("cargo-atcoder-test-new-skip-warmup-bins-for-missing-ws")?;

    assert_no_manifest(tempdir.path());

    cargo_atcoder_new(
        tempdir.path(),
        &[
            "abc999",
            "--skip-warmup",
            "--problems",
            "v",
            "w",
            "x",
            "y",
            "z",
        ],
        &r#"
warning: No existing workspace found. We recommend that you manage packages in one workspace to reduce build time and disk usage. Run `cargo atcoder migrate` to unify existing workspaces. For further information about workspaces, see https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html
     Created binary (application) `abc999` package
     Removed the `main.rs` in `abc999`
       Added 5 `bin`(s) to `abc999`
    Modified `abc999` successfully
    Skipping warming up
"#[1..],
    )?;

    let metadata = cargo_metadata(&tempdir.path().join("abc999").join("Cargo.toml"))?;

    assert_eq!(tempdir.path().join("abc999"), metadata.workspace_root);
    assert_is_git_root(&metadata.workspace_root);
    assert_build_cache_not_exist(&metadata.workspace_root);
    assert_bin_names(
        find_member(&metadata, "abc999"),
        &btreemap!(
            "abc999-v" => Path::new("src").join("bin").join("v.rs"),
            "abc999-w" => Path::new("src").join("bin").join("w.rs"),
            "abc999-x" => Path::new("src").join("bin").join("x.rs"),
            "abc999-y" => Path::new("src").join("bin").join("y.rs"),
            "abc999-z" => Path::new("src").join("bin").join("z.rs"),
        ),
    );

    tempdir.close().map_err(Into::into)
}

fn assert_no_manifest(dir: &Path) {
    if let Some(manifest_dir) = dir.ancestors().find(|p| p.join("Cargo.toml").exists()) {
        panic!("found Cargo.toml at {}", manifest_dir.display());
    }
}

fn cargo_atcoder_new(dir: &Path, args: &[&str], expected_stderr: &str) -> anyhow::Result<()> {
    assert_cmd::Command::cargo_bin("cargo-atcoder")?
        .args(&["atcoder", "new"])
        .args(args)
        .env("CARGO_ATCODER_TEST_CONFIG_DIR", dir)
        .env("CARGO_ATCODER_TEST_CACHE_DIR", dir)
        .current_dir(dir)
        .timeout(TIMEOUT)
        .assert()
        .success()
        .stdout("")
        .stderr(expected_stderr.to_owned());
    Ok(())
}

fn cargo_metadata(manifest_path: &Path) -> cargo_metadata::Result<Metadata> {
    MetadataCommand::new().manifest_path(manifest_path).exec()
}

fn find_member<'a>(metadata: &'a Metadata, name: &str) -> &'a Package {
    metadata
        .packages
        .iter()
        .find(|p| metadata.workspace_members.contains(&p.id) && p.name == name)
        .unwrap_or_else(|| {
            panic!(
                "{}: `{}` not found",
                metadata.workspace_root.display(),
                name,
            )
        })
}

fn assert_is_git_root(workspace_root: &Path) {
    assert!(workspace_root.join(".git").is_dir());
}

fn assert_build_cache_not_exist(workspace_root: &Path) {
    assert!(!workspace_root.join("target").exists());
}

fn assert_bin_names(package: &Package, bins: &BTreeMap<&str, PathBuf>) {
    let mut actual_bins = btreemap!();
    for target in &package.targets {
        assert_eq!(vec!["bin"], target.kind);
        let path = target
            .src_path
            .strip_prefix(package.manifest_path.parent().unwrap())
            .unwrap()
            .to_owned();
        actual_bins.insert(&*target.name, path);
    }
    assert_eq!(*bins, actual_bins);
}
