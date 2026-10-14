pub mod alpha;
pub mod beta;
pub mod gamma;

use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result, ensure};

use crate::{config::AgentConfig, rag::KnowledgeBase};

/// Assemble the same agent network for both MCP and desktop callers.
pub fn build(
    configs: &HashMap<String, AgentConfig>,
    rag: Arc<dyn KnowledgeBase>,
) -> Result<rig::Agent> {
    let config = |name: &str| -> Result<&AgentConfig> {
        let config = configs
            .get(name)
            .with_context(|| format!("Missing agents.{name} configuration"))?;
        ensure!(
            !config.model.trim().is_empty(),
            "agents.{name}.model must not be empty"
        );
        Ok(config)
    };
    let beta = beta::new(config("beta")?).context("Failed to initialize Beta")?;
    let gamma = gamma::new(config("gamma")?, rag).context("Failed to initialize Gamma")?;
    alpha::new(config("alpha")?, vec![beta, gamma]).context("Failed to initialize Alpha")
}
