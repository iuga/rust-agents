use rig::prelude::*;
use rig::providers::ollama;
use rust_agent::Server;
use rust_agent::config::Settings;
use rust_agent::rag::Rag;
use rust_agent::storage::Storage;
use std::sync::Arc;

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

    let embedfn = move |text: &str| -> Vec<f64> {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                embed_model
                    .embed_text(text)
                    .await
                    .expect("Embedding failed")
                    .vec
            })
        })
    };

    let mut rag = Rag::new(
        settings.knowledge.folders,
        &settings.knowledge.include,
        &settings.knowledge.exclude,
        storage,
        Box::new(embedfn),
    );
    let _ = rag.keep_fresh().await;

    Server::new(settings.server.host).mcp(Arc::new(rag)).await?;

    Ok(())
}
