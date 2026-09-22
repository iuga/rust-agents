use anyhow::Result;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio::signal;
use tokio_util::sync::CancellationToken;

mod agents;
mod mcp;
pub mod rag;
pub mod storage;
mod tools;

pub struct Server {
    addr: String,
}

impl Server {
    pub fn new(addr: String) -> Self {
        Server { addr: addr }
    }

    pub async fn mcp(&self) -> Result<(), anyhow::Error> {
        let cancellation_token = CancellationToken::new();

        let service: StreamableHttpService<mcp::MCPServer, LocalSessionManager> =
            StreamableHttpService::new(
                || Ok(mcp::MCPServer::new()),
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
