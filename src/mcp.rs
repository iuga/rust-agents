use crate::{agents, rag::KnowledgeBase};
use rig::completion::Prompt;
use rmcp::{
    ErrorData as McpError, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, ContentBlock},
    model::{ServerCapabilities, ServerInfo},
    schemars, tool, tool_handler, tool_router,
};
use std::sync::Arc;

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
struct ChatArguments {
    #[schemars(
        description = "Conversation ID. Reuse the same ID to continue an existing discussion or task; use a new ID for an unrelated request."
    )]
    id: String,
    #[schemars(
        description = "Task to delegate and the desired outcome. Include relevant context, target entities or services, environment, identifiers, constraints, and how to determine completion. Describe what should be accomplished and any required steps or output format."
    )]
    message: String,
}

#[derive(Clone)]
pub struct MCPServer {
    tool_router: ToolRouter<Self>,
    agent: rig::Agent,
}

impl MCPServer {
    pub fn new(rag: Arc<dyn KnowledgeBase>) -> Self {
        let gamma = agents::gamma::new(Arc::clone(&rag)).unwrap();
        let subagents = vec![gamma];

        let agent = agents::alpha::new(subagents).unwrap();

        Self {
            tool_router: Self::tool_router(),
            agent,
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
        Parameters(ChatArguments { id, message }): Parameters<ChatArguments>,
    ) -> Result<CallToolResult, McpError> {
        println!("[agent-call] Query [{}]: {}", id, message);

        let response = self
            .agent
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
