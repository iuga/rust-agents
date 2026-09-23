use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use rig::tool::Tool;
use serde::Deserialize;
use serde_json::json;

use crate::rag::KnowledgeBase;

pub struct Knowledge {
    rag: Arc<dyn KnowledgeBase>,
}

impl Knowledge {
    pub fn new(rag: Arc<dyn KnowledgeBase>) -> Self {
        Self { rag }
    }
}

#[derive(Deserialize)]
pub struct KnowledgeArgs {
    query: String,
}

#[derive(Debug, thiserror::Error)]
#[error("Knowledge query failed: {0:#}")]
pub struct KnowledgeError(#[from] anyhow::Error);

impl Tool for Knowledge {
    const NAME: &'static str = "knowledge";
    type Error = KnowledgeError;
    type Args = KnowledgeArgs;
    type Output = String;

    fn description(&self) -> String {
        "Search indexed company documentation for context and evidence needed to complete a task. Use this tool to find business rules, terminology, system descriptions, and documented procedures. Returns document excerpts with source filenames for you to evaluate and reference. Results are selected by semantic similarity; assess their relevance to the task. Documentation describes recorded knowledge and expected behavior, not a live observation of a service's current state."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "A short, precise query targeting one specific fact, concept, or procedure. Use the exact entity or service name and only the terms needed to distinguish the information sought. Omit background context, conversation history, task instructions, and unrelated details. Use separate queries for separate information needs."
                }
            },
            "required": ["query"],
        })
    }

    async fn call(
        &self,
        _context: &mut rig::tool::ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let started = Instant::now();
        eprintln!("[tool-call] Knowledge query: {}", args.query);

        let chunks = self.rag.query(&args.query).await.map_err(|error| {
            eprintln!(
                "[tool-error] Knowledge query {:?} failed after {:?}: {error:#}",
                args.query,
                started.elapsed(),
            );
            KnowledgeError(error)
        })?;
        eprintln!(
            "[tool-result] Knowledge query {:?}: {} chunks in {:?}",
            args.query,
            chunks.len(),
            started.elapsed(),
        );
        if chunks.is_empty() {
            return Ok("No matching documents found.".to_string());
        }

        Ok(chunks
            .into_iter()
            .map(|chunk| format!("Source: {}\n{}", chunk.filename, chunk.content))
            .collect::<Vec<_>>()
            .join("\n\n"))
    }
}
