use std::collections::HashMap;

use anyhow::{Context, Result};
use config::Config;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub agents: HashMap<String, AgentConfig>,
    pub knowledge: Knowledge,
    pub server: Server,
}

#[derive(Debug, Deserialize)]
pub struct AgentConfig {
    pub model: String,
    pub skills: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Knowledge {
    pub include: String,
    pub exclude: String,
    pub database_uri: String,
    pub model: String,
    pub folders: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct Server {
    pub host: String,
}

impl Settings {
    pub fn build() -> Result<Self> {
        let settings: Self = Config::builder()
            .add_source(config::File::with_name("settings"))
            .add_source(config::Environment::with_prefix("APP"))
            .build()
            .context("Failed to load settings; provide settings.yaml (see examples.yaml)")?
            .try_deserialize()
            .context("Invalid settings configuration")?;
        Ok(settings)
    }
}
