use anyhow::Result;
use config::Config;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Settings {
    pub agents: Agents,
    pub knowledge: Knowledge,
    pub server: Server,
}

#[derive(Debug, Deserialize)]
pub struct Agents {
    pub model: String,
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
            .add_source(config::File::with_name("settings")) //`./settings.toml`
            .add_source(config::Environment::with_prefix("APP"))
            .build()?
            .try_deserialize()?;
        Ok(settings)
    }
}
