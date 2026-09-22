use rig::prelude::*;
use rig::providers::ollama;
use rust_agent::Server;
use rust_agent::rag::Rag;
use rust_agent::storage::Storage;


#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {

    let mut storage = Storage::new();
    if let Err(err) = storage.connect("data.db").await {
        panic!("Failed to connect to the database: {}", err);
    };
    
    let client = ollama::Client::from_env()?;
    let embed_model = client.embedding_model("qwen3-embedding:8b");

    let paths = vec!["/Users/estebandelboca/bcroot/Obsidian/Bluecore/".to_string()];
    let rag = Rag::new(paths, ".*md", ".*porygon.*", embed_model, storage);
    if let Err(err) = rag.update().await {
        println!("Error updating the RAG: {}", err);
    }

    Server::new(String::from("127.0.0.1:8000"))
        .mcp()
        .await?;
    Ok(())
}
