use cargo_metadata::{Metadata, MetadataCommand, Package, Target};
use difference::assert_diff;
use pretty_assertions::assert_eq;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tempdir::TempDir;

const TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn from_old() -> anyhow::Result<()> {
    return test(
        "cargo-atcoder-test-migrate-from-new",
        package_minifest_orig,
        package_minifest_edit,
    );

    fn package_minifest_orig(package_name: &str) -> String {
        format!(
            r#"[package]
name = "{}"
version = "0.1.0"
edition = "2018"

[profile.release]
lto = true
panic = 'abort'
"#,
            package_name,
        )
    }

    fn package_minifest_edit(package_name: &str) -> String {
        format!(
            r#"[package]
name = "{package_name}"
version = "0.1.0"
edition = "2018"

[package.metadata.cargo-atcoder.problems]
a = {{bin = "{bin_name}"}}

[[bin]]
name = "{bin_name}"
path = "./src/bin/a.rs"
"#,
            package_name = package_name,
            bin_name = format!("{}-a", package_name),
        )
    }
}

#[test]
fn from_current() -> anyhow::Result<()> {
    return test(
        "cargo-atcoder-test-migrate-from-current",
        package_minifest_orig,
        package_minifest_edit,
    );

    fn package_minifest_orig(package_name: &str) -> String {
        format!(
            r#"[package]
name = "{package_name}"
version = "0.1.0"
edition = "2018"

[package.metadata.cargo-atcoder.problems]
a = {{bin = "{bin_name}"}}

[[bin]]
name = "{bin_name}"
path = "./src/bin/a.rs"

[profile.release]
lto = true
panic = 'abort'
"#,
            package_name = package_name,
            bin_name = format!("{}-a", package_name),
        )
    }

    fn package_minifest_edit(package_name: &str) -> String {
        format!(
            r#"[package]
name = "{package_name}"
version = "0.1.0"
edition = "2018"

[package.metadata.cargo-atcoder.problems]
a = {{bin = "{bin_name}"}}

[[bin]]
name = "{bin_name}"
path = "./src/bin/a.rs"
"#,
            package_name = package_name,
            bin_name = format!("{}-a", package_name),
        )
    }
}

fn test(
    tempdir_prefix: &str,
    package_manifest_orig: fn(&str) -> String,
    package_manifest_edit: fn(&str) -> String,
) -> anyhow::Result<()> {
    let tempdir = TempDir::new(tempdir_prefix)?;
    let dir_canonicalized = dunce::canonicalize(tempdir.path())?;

    assert_no_manifest(&dir_canonicalized);

    for &package_name in &["contest1", "contest2"] {
        fs::create_dir_all(dir_canonicalized.join(package_name).join("src").join("bin"))?;

        fs::write(
            dir_canonicalized.join(package_name).join("Cargo.toml"),
            package_manifest_orig(package_name).to_string(),
        )?;

        fs::write(
            dir_canonicalized
                .join(package_name)
                .join("src")
                .join("bin")
                .join("a.rs"),
            r#"fn main() {
    todo!();
}
"#,
        )?;
    }

    assert_cmd::Command::cargo_bin("cargo-atcoder")?
        .args(&["atcoder", "migrate", "--noconfirm"])
        .arg(&dir_canonicalized)
        .env("CARGO_ATCODER_TEST_CONFIG_DIR", &dir_canonicalized)
        .env("CARGO_ATCODER_TEST_CACHE_DIR", &dir_canonicalized)
        .current_dir(&dir_canonicalized)
        .timeout(TIMEOUT)
        .assert()
        .success()
        .stdout("")
        .stderr(format!(
            r#"       Found `{contest1_manifest}`
       Found `{contest2_manifest}`
       Wrote `{root_manifest}`
       Wrote `{contest1_manifest}`
       Wrote `{contest2_manifest}`
"#,
            root_manifest = dir_canonicalized.join("Cargo.toml").display(),
            contest1_manifest = dir_canonicalized
                .join("contest1")
                .join("Cargo.toml")
                .display(),
            contest2_manifest = dir_canonicalized
                .join("contest2")
                .join("Cargo.toml")
                .display(),
        ));

    assert_diff!(
        &r#"
[workspace]
members = ["contest1", "contest2"]

[profile.release]
lto = true
panic = 'abort'
"#[1..],
        &fs::read_to_string(dir_canonicalized.join("Cargo.toml"))?,
        "\n",
        0
    );

    let metadata = cargo_metadata_no_deps(&dir_canonicalized.join("Cargo.toml"))?;

    assert_eq!(
        &["contest1", "contest2"],
        &*metadata
            .packages
            .iter()
            .map(|Package { name, .. }| name)
            .collect::<Vec<_>>(),
    );

    assert_eq!(
        &["contest1-a", "contest2-a"],
        &*metadata
            .packages
            .iter()
            .flat_map(|Package { targets, .. }| targets)
            .map(|Target { name, .. }| name)
            .collect::<Vec<_>>(),
    );

    for package_name in &["contest1", "contest2"] {
        assert_diff!(
            &package_manifest_edit(package_name).to_string(),
            &fs::read_to_string(dir_canonicalized.join(package_name).join("Cargo.toml"))?,
            "\n",
            0
        );
    }

    Ok(())
}

fn assert_no_manifest(dir: &Path) {
    if let Some(manifest_dir) = dir.ancestors().find(|p| p.join("Cargo.toml").exists()) {
        panic!("found Cargo.toml at {}", manifest_dir.display());
    }
}

fn cargo_metadata_no_deps(manifest_path: &Path) -> cargo_metadata::Result<Metadata> {
    MetadataCommand::new()
        .manifest_path(manifest_path)
        .no_deps()
        .exec()
}
