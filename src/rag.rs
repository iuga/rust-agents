use crate::storage::{DocumentRow, Storage};
use anyhow::{Context, Result};
use futures::future::BoxFuture;
use regex::Regex;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::mpsc;
use walkdir::{DirEntry, WalkDir};

type EmbeddingGenerator = Box<dyn Fn(&str) -> Vec<f64> + Send + Sync + 'static>;

/// Model-independent, shared access to knowledge retrieval.
pub trait KnowledgeBase: Send + Sync {
    fn query<'a>(&'a self, query: &'a str) -> BoxFuture<'a, Result<Vec<DocumentChunk>>>;
}

#[derive(Debug)]
struct Message {
    filename: String,
    content: String,
    hash: String,
}

#[derive(Debug)]
pub struct DocumentChunk {
    pub filename: String,
    pub content: String,
}

impl From<DocumentRow> for DocumentChunk {
    fn from(dr: DocumentRow) -> Self {
        Self {
            filename: dr.filename,
            content: dr.content,
        }
    }
}

pub struct Rag {
    paths: Vec<String>,
    include: Regex,
    exclude: Regex,
    supported: Regex,
    embedfn: Arc<EmbeddingGenerator>,
    storage: Arc<Storage>,
}

impl KnowledgeBase for Rag {
    fn query<'a>(&'a self, query: &'a str) -> BoxFuture<'a, Result<Vec<DocumentChunk>>> {
        Box::pin(Rag::query(self, query))
    }
}

impl Rag {
    pub fn new(
        paths: Vec<String>,
        include: &str,
        exclude: &str,
        storage: Storage,
        embedfn: EmbeddingGenerator,
    ) -> Self {
        Rag {
            include: Regex::new(include).unwrap(),
            exclude: Regex::new(exclude).unwrap(),
            supported: Regex::new(".*.md").unwrap(),
            storage: Arc::new(storage),
            embedfn: Arc::new(embedfn),
            paths,
        }
    }

    pub async fn query(&self, query: &str) -> Result<Vec<DocumentChunk>> {
        let query_embedding = (self.embedfn)(query);
        let dimensions = query_embedding.len();
        let answers = self
            .storage
            .query(query_embedding)
            .await
            .with_context(|| {
                format!(
                    "Failed to search knowledge storage with {dimensions}-dimensional embedding"
                )
            })?
            .into_iter()
            .map(DocumentChunk::from)
            .collect();
        Ok(answers)
    }

    pub async fn keep_fresh(&mut self) -> Result<()> {
        let (tx_files, mut rx_files) = mpsc::channel::<Message>(50);
        let (tx_content, mut rx_content) = mpsc::channel::<Message>(50);

        let paths = self.paths.to_vec();
        let include = self.include.clone();
        let exclude = self.exclude.clone();
        let supported = self.supported.clone();

        let storage_check = Arc::clone(&self.storage);
        let storage_save = Arc::clone(&self.storage);
        let embedfn = Arc::clone(&self.embedfn);

        //
        // Filenames to index Producer...
        //
        tokio::spawn(async move {
            for path in paths.iter() {
                let pit = WalkDir::new(path)
                    .into_iter()
                    .filter_entry(|e| !Self::is_hidden(e))
                    .filter_map(|e| e.ok());

                for entry in pit {
                    if entry.path().is_dir() {
                        continue;
                    }
                    let filename = entry.path().display().to_string();
                    if exclude.is_match(&filename) {
                        continue;
                    }
                    if include.is_match(&filename) && supported.is_match(&filename) {
                        let msg = Message {
                            filename: filename.to_string(),
                            content: String::new(),
                            hash: String::new(),
                        };
                        tx_files.send(msg).await.expect("Consumer stopped");
                    }
                }
            }
        });

        //
        // Consumer of filenames, and parser...
        //
        tokio::spawn(async move {
            while let Some(msg) = rx_files.recv().await {
                if let Ok(content) = fs::read_to_string(&msg.filename) {
                    let fhash = Self::hasher(&msg.content);
                    match storage_check.get_by_filename(&msg.filename).await {
                        Ok(stored_docs) => {
                            for sdoc in stored_docs {
                                if sdoc.hash == fhash {
                                    continue;
                                }
                            }
                            let msg = Message {
                                filename: msg.filename.to_string(),
                                hash: fhash,
                                content,
                            };
                            println!("[rag] Updating: {:?}", msg.filename);
                            tx_content
                                .send(msg)
                                .await
                                .expect("Content consumer stopped");
                        }
                        Err(err) => {
                            println!("[err] loading the doc from storage: {}", err);
                        }
                    }
                }
            }
        });

        //
        // Consumer of Content, chunks and embeddings ...
        //
        tokio::spawn(async move {
            while let Some(msg) = rx_content.recv().await {
                let _ = storage_save.delete_by_filename(&msg.filename).await;
                for chunk in Self::chunks(&msg.filename, &msg.content) {
                    let text: String = chunk.into_iter().collect();
                    let embedding = embedfn(&text);
                    let row = DocumentRow::new(&msg.filename, &text, embedding, &msg.hash);
                    if let Err(err) = storage_save.insert(&row).await {
                        println!("> error: {}", err);
                    };
                }
            }
        });

        Ok(())
    }

    fn chunks(filename: &str, content: &str) -> Vec<Vec<char>> {
        let extension = Path::new(filename).extension().and_then(|ext| ext.to_str());

        let builder = match extension {
            Some("md") => chunkedrs::chunk(content).markdown(),
            Some(_) | None => chunkedrs::chunk(content).max_tokens(256).overlap(50),
        };

        builder
            .split()
            .iter()
            .map(|c| {
                format!("{}\n{}", c.section_path.join(" "), c.content)
                    .chars()
                    .collect::<Vec<char>>()
            })
            .collect::<Vec<Vec<char>>>()
    }

    fn is_hidden(entry: &DirEntry) -> bool {
        entry
            .file_name()
            .to_str()
            .map(|s| s.starts_with("."))
            .unwrap_or(false)
    }

    fn hasher(content: &str) -> String {
        let digest = Sha256::digest(content.as_bytes());
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}
