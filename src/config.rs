use anyhow::{Context as _, Result};
use serde::Deserialize;
use std::path::PathBuf;
use std::{env, fs};
use toml::Value;
use toml_edit::Document;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub atcoder: AtCoder,
    pub profile: Profile,
    pub dependencies: Value,
    pub project: Project,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AtCoder {
    pub submit_via_binary: bool,
    pub use_cross: bool,
    pub binary_column: usize,
    pub update_interval: u64,
    pub strip_path: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Profile {
    pub target: String,
    pub release: Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Project {
    pub template: String,
    pub rustc_version: Option<String>,
}

const DEFAULT_CONFIG_STR: &str = include_str!("../config/cargo-atcoder.toml");

fn config_path() -> Result<PathBuf> {
    let config_path = if let Some(path) = env::var_os("CARGO_ATCODER_TEST_CONFIG_DIR") {
        path.into()
    } else {
        dirs::config_dir().with_context(|| "Failed to get config directory")?
    }
    .join("cargo-atcoder.toml");

    if !config_path.exists() {
        fs::create_dir_all(config_path.parent().unwrap())?;
        fs::write(&config_path, DEFAULT_CONFIG_STR)?;
    }

    Ok(config_path)
}

pub fn read_config() -> Result<Config> {
    let config_path = config_path()?;
    let s = fs::read_to_string(&config_path)
        .with_context(|| format!("Failed to read: `{}`", config_path.display()))?;
    toml::from_str(&s).with_context(|| {
        format!(
            "Failed to parse the TOML file at `{}`",
            config_path.display(),
        )
    })
}

pub fn read_config_preserving() -> Result<Document> {
    let config_path = config_path()?;
    Ok(fs::read_to_string(&config_path)?.parse::<Document>()?)
}
