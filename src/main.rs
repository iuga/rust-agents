use std::sync::Arc;

use config::Config;
use rig::prelude::*;
use rig::providers::ollama;
use rust_agent::Server;
use rust_agent::rag::Rag;
use rust_agent::storage::Storage;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let settings = Config::builder()
        .add_source(config::File::with_name("settings")) //`./settings.toml`
        .add_source(config::Environment::with_prefix("APP"))
        .build()
        .unwrap();

    let database_uri = settings.get_string("knowledge.database_uri").unwrap();
    let knowledge_model = settings.get_string("knowledge.model").unwrap();
    let knowledge_include = settings.get_string("knowledge.include").unwrap();
    let knowledge_exclude = settings.get_string("knowledge.exclude").unwrap();
    let server_host = settings.get_string("server.host").unwrap();
    let knowledge_paths = settings
        .get_array("knowledge.folders")
        .unwrap()
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<String>>();

    let storage = Storage::build(&database_uri).await;
    if storage.is_err() {
        panic!("Failed to connect to the database: {:?}", storage.err());
    };
    let storage = storage.unwrap();

    let client = ollama::Client::from_env()?;
    let embed_model = client.embedding_model(knowledge_model);

    let mut rag = Rag::new(
        knowledge_paths,
        &knowledge_include,
        &knowledge_exclude,
        embed_model,
        storage,
    );
    if let Err(err) = rag.update().await {
        panic!("Error updating the RAG: {}", err);
    }

    Server::new(server_host).mcp(Arc::new(rag)).await?;

    Ok(())
}
