use anyhow::Result;
use regex::Regex;
use rig::embeddings::EmbeddingModel;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, fs};
use walkdir::{DirEntry, WalkDir};

use crate::storage::{DocumentRow, Storage};

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

pub struct Rag<T: EmbeddingModel> {
    paths: Vec<String>,
    include: Regex,
    exclude: Regex,
    supported: Regex,
    model: T,
    storage: Storage,
    hashes: HashMap<String, String>,
}

impl<T: EmbeddingModel> Rag<T> {
    pub fn new(
        paths: Vec<String>,
        include: &str,
        exclude: &str,
        model: T,
        storage: Storage,
    ) -> Self {
        Rag {
            paths,
            include: Regex::new(include).unwrap(),
            exclude: Regex::new(exclude).unwrap(),
            supported: Regex::new(".*.md").unwrap(),
            model: model,
            storage: storage,
            hashes: HashMap::new(),
        }
    }

    pub async fn query(&self, query: &str) -> Result<Vec<DocumentChunk>> {
        let query_embedding = self.model.embed_text(query).await?;
        let answers = self
            .storage
            .query(query_embedding.vec)
            .await?
            .into_iter()
            .map(DocumentChunk::from)
            .collect();
        Ok(answers)
    }

    pub async fn update(&mut self) -> Result<()> {
        println!("[rag] Updating...");

        let mut documents: Vec<DocumentRow> = Vec::new();
        for filename in self.collect() {
            if let Ok(content) = fs::read_to_string(&filename) {
                let hash = self.hasher(&content);

                if !self.should_we_update(&filename, &hash).await {
                    continue;
                }

                println!("~> Indexing file {filename}");
                self.storage.delete_by_filename(&filename).await?;

                for chunk in self.chunks(&content) {
                    let text: String = chunk.into_iter().collect();
                    let embedding = self.model.embed_text(&text).await?;
                    let row = DocumentRow::new(
                        &filename,
                        &embedding.document.clone(),
                        embedding.vec.clone(),
                        &hash,
                    );
                    if let Err(err) = self.storage.insert(&row).await {
                        println!("> error: {}", err);
                    };
                    documents.push(row);
                }

                self.hashes.insert(filename.to_string(), hash.to_string());
            }
        }

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

    async fn should_we_update(&mut self, filename: &str, hash: &str) -> bool {
        if self
            .hashes
            .get(filename)
            .is_some_and(|value| *value == hash)
        {
            return false;
        }
        let docs = self.storage.get_by_filename(&filename).await;
        match docs {
            Ok(stored_docs) => {
                for sdoc in stored_docs {
                    if sdoc.hash == hash {
                        return false;
                    }
                }
                return true;
            }
            Err(err) => {
                println!("[err] loading the doc from storage: {}", err);
                true
            }
        }
    }
}
