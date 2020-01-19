use anyhow::{anyhow, Result};
use serde_derive::Deserialize;
use std::collections::BTreeMap;
use toml::Value;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub profile: Profile,
    pub dependencies: BTreeMap<String, Value>,
    pub project: Project,
}

#[derive(Debug, Deserialize)]
pub struct Profile {
    pub target: String,
    pub release: Value,
}

#[derive(Debug, Deserialize)]
pub struct Project {
    pub template: String,
    // pub rustc_version: String,
}

pub fn read_config() -> Result<Config> {
    let config_path = dirs::config_dir()
        .ok_or(anyhow!("Failed to get config directory"))?
        .join("cargo-atcoder.toml");
    let s = std::fs::read_to_string(&config_path)
        .map_err(|_| anyhow!("Cannot read file: {}", config_path.display()))?;
    let config: Config = toml::from_str(&s)?;
    dbg!(&config);

    Ok(config)
}
