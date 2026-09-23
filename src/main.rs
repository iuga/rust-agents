use std::sync::Arc;

use rig::prelude::*;
use rig::providers::ollama;
use rust_agent::Server;
use rust_agent::config::Settings;
use rust_agent::rag::Rag;
use rust_agent::storage::Storage;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let settings = Settings::build()?;
    // println!("[config] {:?}", settings);

    let storage = Storage::build(&settings.knowledge.database_uri).await;
    if storage.is_err() {
        panic!("Failed to connect to the database: {:?}", storage.err());
    };
    let storage = storage.unwrap();

    let client = ollama::Client::from_env()?;
    let embed_model = client.embedding_model(&settings.knowledge.model);

    let mut rag = Rag::new(
        settings.knowledge.folders,
        &settings.knowledge.include,
        &settings.knowledge.exclude,
        embed_model,
        storage,
    );
    if let Err(err) = rag.update().await {
        panic!("Error updating the RAG: {}", err);
    }

    Server::new(settings.server.host).mcp(Arc::new(rag)).await?;

    Ok(())
}
