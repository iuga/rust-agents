use std::convert::Infallible;

use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

pub struct GenerateUuid;

#[derive(Deserialize)]
pub struct GenerateUuidArgs {}

impl Tool for GenerateUuid {
    const NAME: &'static str = "generate-uuid";
    type Error = Infallible;
    type Args = GenerateUuidArgs;
    type Output = String;

    fn description(&self) -> String {
        "Generate a fresh UUID v7. Use this tool whenever asked to generate a UUID and return its result unchanged."
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        _args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let id = Uuid::now_v7().to_string();
        println!("[tool-call] Genearate UUID: {}", id);
        Ok(id)
    }
}
