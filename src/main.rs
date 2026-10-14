use rust_agent::Server;
use rust_agent::config::Settings;
use rust_agent::rag::Rag;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let settings = Settings::build()?;
    let server = Server::new(settings.server.host, settings.agents)?;
    let rag = Rag::build(settings.knowledge).await?;

    server.mcp(Arc::new(rag)).await?;

    Ok(())
}
