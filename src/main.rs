use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use tokio_util::sync::CancellationToken;
use tokio::signal;

mod agents;
mod mcp;
mod tools;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {

    let cancellation_token = CancellationToken::new();

    let service: StreamableHttpService<mcp::MCPServer, LocalSessionManager> =
        StreamableHttpService::new(
            || Ok(mcp::MCPServer::new()),
            LocalSessionManager::default().into(),
            StreamableHttpServerConfig::default()
                .with_cancellation_token(cancellation_token.child_token()),
        );

    let app = axum::Router::new().nest_service("/mcp", service);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:8000").await?;

    println!("MCP server listening on http://127.0.0.1:8000/mcp");

    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            signal::ctrl_c().await.ok();
            cancellation_token.cancel();
        })
        .await?;

    Ok(())
}
