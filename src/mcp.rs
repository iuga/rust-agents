use std::{collections::HashMap, sync::Arc};

use crate::agents::{self};
use rig::completion::Prompt;
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
};
use tokio::sync::Mutex;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ChatArguments {
    #[schemars(description = "ID of the conversation to send the message to.")]
    id: String,
    #[schemars(
        description = "The profile or kind of agent you want to talk with: aplha or beta (default: alpha)"
    )]
    profile: String,
    #[schemars(
        description = "Question or request for the expert agent. Include relevant context, constraints, and the desired response format."
    )]
    message: String,
}

#[derive(Clone)]
pub struct MCPServer {
    tool_router: ToolRouter<Self>,
    agents: Arc<Mutex<HashMap<String, rig::Agent>>>,
}

impl MCPServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
            agents: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MCPServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
    }
}

#[tool_router]
impl MCPServer {
    #[tool(
        description = "Send a question or request to an expert agent in the specified conversation. Use this tool when specialized knowledge or domain-specific analysis is needed."
    )]
    async fn chat(
        &self,
        Parameters(ChatArguments {
            id,
            profile,
            message,
        }): Parameters<ChatArguments>,
    ) -> Result<CallToolResult, McpError> {
        let mut agents = self.agents.lock().await;
        let agent = agents.entry(id.to_string()).or_insert_with(|| {
            match profile.as_str() {
                "alpha" => agents::alpha::new().unwrap(),
                // "beta" => agents::beta::new().unwrap(),
                _ => agents::alpha::new().unwrap(),
            }
        });
        let response = agent.prompt(message.to_string()).conversation(id).await;
        match response {
            Ok(answer) => Ok(CallToolResult::success(vec![ContentBlock::text(
                answer.to_string(),
            )])),
            Err(e) => Ok(CallToolResult::error(vec![ContentBlock::text(format!(
                "error generating a response: {e}"
            ))])),
        }
    }
}
