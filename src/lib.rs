use std::{collections::HashMap, sync::Arc};

use anyhow::{Context, Result, ensure};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio::signal;
use tokio_util::sync::CancellationToken;

mod agents;
pub mod config;
mod mcp;
pub mod rag;
pub mod skills;
pub mod storage;
mod tools;

pub struct Server {
    addr: String,
    agents: HashMap<String, config::AgentConfig>,
}

impl Server {
    pub fn new(addr: String, agents: HashMap<String, config::AgentConfig>) -> Result<Self> {
        // The current agent topology requires these roles; configuration keys stay extensible.
        for name in ["alpha", "beta", "gamma"] {
            let config = agents
                .get(name)
                .with_context(|| format!("Missing agents.{name} configuration"))?;
            ensure!(
                !config.model.trim().is_empty(),
                "agents.{name}.model must not be empty"
            );
        }
        Ok(Server { addr, agents })
    }

    pub async fn mcp(self, rag: Arc<dyn rag::KnowledgeBase>) -> Result<(), anyhow::Error> {
        let cancellation_token = CancellationToken::new();

        let service: StreamableHttpService<mcp::MCPServer, LocalSessionManager> =
            StreamableHttpService::new(
                move || {
                    mcp::MCPServer::new(Arc::clone(&rag), &self.agents)
                        .map_err(std::io::Error::other)
                },
                LocalSessionManager::default().into(),
                StreamableHttpServerConfig::default()
                    .with_cancellation_token(cancellation_token.child_token()),
            );

        let app = axum::Router::new().nest_service("/mcp", service);

        let listener = tokio::net::TcpListener::bind(self.addr.as_str()).await?;

        println!("MCP server listening on {}/mcp", self.addr);

        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                signal::ctrl_c().await.ok();
                cancellation_token.cancel();
            })
            .await?;

        Ok(())
    }
}
