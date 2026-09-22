use rig::prelude::*;
use rig::providers::ollama;
use rust_agent::Server;
use rust_agent::rag::Rag;
use rust_agent::storage::Storage;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let storage = Storage::build("data.db").await;
    if storage.is_err() {
        panic!("Failed to connect to the database: {:?}", storage.err());
    };
    let storage = storage.unwrap();

    let client = ollama::Client::from_env()?;
    let embed_model = client.embedding_model("qwen3-embedding:8b");

    let paths = vec!["/Users/estebandelboca/bcroot/Obsidian/Bluecore/".to_string()];
    let mut rag = Rag::new(paths, ".*md", ".*porygon.*", embed_model, storage);
    if let Err(err) = rag.update().await {
        println!("Error updating the RAG: {}", err);
    }

    let answers = rag.query("What are the Porygon features?").await?;
    for answer in answers {
        println!("> {answer:?}");
    }

    Server::new(String::from("127.0.0.1:8000")).mcp().await?;
    Ok(())
}
