use std::{collections::HashMap, sync::Arc};

use crate::{agents, rag::KnowledgeBase};
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
    #[schemars(
        description = "Conversation ID. Reuse the same ID to continue an existing discussion or task; use a new ID for an unrelated request."
    )]
    id: String,
    #[schemars(
        description = "Specialist agent profile to use. Currently supported: alpha, the company business-domain specialist."
    )]
    profile: String,
    #[schemars(
        description = "Task to delegate and the desired outcome. Include relevant context, target entities or services, environment, identifiers, constraints, and how to determine completion. Describe what should be accomplished and any required steps or output format."
    )]
    message: String,
}

#[derive(Clone)]
pub struct MCPServer {
    tool_router: ToolRouter<Self>,
    agents: Arc<Mutex<HashMap<String, rig::Agent>>>,
    rag: Arc<dyn KnowledgeBase>,
}

impl MCPServer {
    pub fn new(rag: Arc<dyn KnowledgeBase>) -> Self {
        Self {
            tool_router: Self::tool_router(),
            agents: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            rag,
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
        description = "Delegate a task to a specialist agent with knowledge of the company's business domain, systems, and processes. The agent uses its available tools and domain knowledge to carry out the task, which may require multiple steps. Tasks may include researching a topic, checking a service's current state, investigating an issue, or performing a supported operation. Provide the desired outcome, relevant context, and constraints. The agent returns its results and reports any blockers or incomplete work. Reuse the conversation ID to continue or refine the same task."
    )]
    async fn chat(
        &self,
        Parameters(ChatArguments {
            id,
            profile,
            message,
        }): Parameters<ChatArguments>,
    ) -> Result<CallToolResult, McpError> {
        println!("[agent-call] Query [{}/{}]: {}", id, profile, message);
        let mut agents = self.agents.lock().await;
        let agent = agents.entry(id.to_string()).or_insert_with(|| {
            match profile.as_str() {
                "alpha" => agents::alpha::new(Arc::clone(&self.rag)).unwrap(),
                // "beta" => agents::beta::new().unwrap(),
                _ => agents::alpha::new(Arc::clone(&self.rag)).unwrap(),
            }
        });
        let response = agent
            .prompt(message.to_string())
            .max_turns(5)
            .conversation(id)
            .await;
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
