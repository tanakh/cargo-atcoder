use anyhow::{anyhow, Result};
use serde_derive::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use toml::Value;

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub atcoder: AtCoder,
    pub profile: Profile,
    pub dependencies: BTreeMap<String, Value>,
    pub project: Project,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AtCoder {
    pub submit_via_binary: bool,
    pub update_interval: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Profile {
    pub target: String,
    pub release: Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Project {
    pub template: String,
    // pub rustc_version: String,
}

const DEFAULT_CONFIG_STR: &str = include_str!("../config/cargo-atcoder.toml");

lazy_static::lazy_static! {
    static ref DEFAULT_CONFIG: Config = toml::from_str(DEFAULT_CONFIG_STR).unwrap();
}

pub fn read_config() -> Result<Config> {
    let config_path = dirs::config_dir()
        .ok_or_else(|| anyhow!("Failed to get config directory"))?
        .join("cargo-atcoder.toml");

    if !config_path.exists() {
        dbg!();
        fs::write(&config_path, DEFAULT_CONFIG_STR)?;
        return Ok(DEFAULT_CONFIG.clone());
    }

    let s = fs::read_to_string(&config_path)
        .map_err(|_| anyhow!("Cannot read file: {}", config_path.display()))?;
    Ok(toml::from_str(&s)?)
}
