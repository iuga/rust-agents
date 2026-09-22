use regex::Regex;
use rig::embeddings::{Embedding, EmbeddingModel};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs};
use uuid::Uuid;
use walkdir::{DirEntry, WalkDir};

use crate::storage::{Storage, DocumentRow};

pub struct Document {
    pub id: Uuid,
    pub filename: String,
    pub hash: String,
    pub embedding: Embedding,
}

impl Document {
    pub fn new(filename: String, embedding: Embedding, hash: String) -> Self {
        Self {
            id: Uuid::now_v7(),
            filename: filename,
            embedding: embedding,
            hash: hash,
        }
    }
}

pub struct Rag<T: EmbeddingModel> {
    paths: Vec<String>,
    include: Regex,
    exclude: Regex,
    supported: Regex,
    model: T,
    storage: Storage,
}

impl<T: EmbeddingModel> Rag<T> {
    pub fn new(paths: Vec<String>, include: &str, exclude: &str, model: T, storage: Storage) -> Self {
        Rag {
            paths,
            include: Regex::new(include).unwrap(),
            exclude: Regex::new(exclude).unwrap(),
            supported: Regex::new(".*.md").unwrap(),
            model: model,
            storage: storage,
        }
    }

    pub async fn update(&self) -> Result<(), String> {
        println!("[rag] Updating...");

        let mut hashes: HashMap<String, String> = HashMap::new();
        let mut documents: Vec<Document> = Vec::new();
        for filename in self.collect() {
            if let Ok(content) = fs::read_to_string(&filename) {
                let hash = self.hasher(&content);

                hashes.insert(filename.clone(), hash.clone()); 

                // Short Circuit
                if !self.should_we_update(&filename, &hash).await{
                    println!("Skipping file {} ({}), already up to date;", &filename, &hash);
                    continue;
                }

                for chunk in self.chunks(&content) {
                    let text: String = chunk.into_iter().collect();
                    let embedding = self.model.embed_text(&text).await;
                    println!("> [{}] {}", &hash, &filename);
                    if embedding.is_ok() {
                        let d = Document::new(
                            filename.clone(),
                            embedding.unwrap(),
                            hash.clone(),
                        );
                        let row = DocumentRow {
                            uuid: d.id.to_string(),
                            filename: d.filename.clone(),
                            hash: d.hash.clone(),
                            content: d.embedding.document.clone(),
                            embedding: d.embedding.vec.clone(),
                        };

                        if let Err(err) = self.storage.insert(&row).await {
                            println!("> error: {}", err);
                        };
                        documents.push(d);
                    }
                }
            }
        }
        println!("[rag] hashes: {:?}", hashes);
        println!("[rag] Updated");
        Ok(())
    }

    /// collect analizes all the paths scanning all files and returning the ones
    /// should be incluided in the knowledge base according the defined rules.
    fn collect(&self) -> Vec<String> {
        let mut files: Vec<String> = Vec::new();
        for path in self.paths.iter() {
            for entry in WalkDir::new(path)
                .into_iter()
                .filter_entry(|e| !self.is_hidden(e))
                .filter_map(|e| e.ok())
            {
                if entry.path().is_dir() {
                    continue;
                }
                let filename = entry.path().display().to_string();
                if self.exclude.is_match(&filename) {
                    continue;
                }
                if self.include.is_match(&filename) && self.supported.is_match(&filename) {
                    files.push(entry.path().display().to_string());
                }
            }
        }
        files
    }

    fn chunks(&self, content: &str) -> Vec<Vec<char>> {
        return content
            .chars()
            .collect::<Vec<char>>()
            .chunks(4096)
            .map(|chunk| chunk.to_vec())
            .collect::<Vec<Vec<char>>>();
    }

    fn is_hidden(&self, entry: &DirEntry) -> bool {
        entry
            .file_name()
            .to_str()
            .map(|s| s.starts_with("."))
            .unwrap_or(false)
    }

    fn hasher(&self, content: &str) -> String {
        let digest = Sha256::digest(content.as_bytes());
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    async fn should_we_update(&self, filename: &str, hash: &str) -> bool {
        let docs = self.storage.get_by_filename(&filename).await;
        match docs {
            Ok(stored_docs) => {
                for sdoc in stored_docs {
                    println!("~> {} vs {}", sdoc.hash, hash);
                    if sdoc.hash == hash {  
                        return false;
                    }
                }
                return true;
            },
            Err(err) => {
                println!("[err] loading the doc from storage: {}", err);
                true
            }
        }
    }
}
