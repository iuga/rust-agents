use rig::providers::ollama;
use rig::prelude::*;
use rig::memory::InMemoryConversationMemory;

use crate::tools::math::Subtract;

const PREAMBLE: &str = "You are an expert in the domain logic of an specific topic: Your company";


pub fn new()-> Option<rig::Agent> {

    let memory = InMemoryConversationMemory::new();

    Some(ollama::Client::from_env().ok()?
        .agent("gemma4")
        .memory(memory)
        .preamble(PREAMBLE)
        .tool(Subtract)
        .build())
}
