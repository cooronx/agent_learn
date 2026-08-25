use serde::{Deserialize, Serialize};



#[derive(Debug,Clone)]
pub struct Model {
    pub id: String,
    pub provider: String,
    pub api: String,
}

#[derive(Debug,Clone,Serialize,Deserialize)]
pub enum ModelMessage {
    User(UserMessage),
    Assistant(AssistantMessage),
    ToolResult(ToolCallResultMessage)
}


#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct UserMessage {
    pub content: String
}


#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct AssistantMessage {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>
}

#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct ToolCallResultMessage {
    pub tool_call_id: String,
    pub content: String,
}

#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

#[derive(Debug,Clone,Serialize,Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: Option<String>,
    pub parameters: serde_json::Value,
}

#[derive(Debug,Clone)]
pub struct Context {
    pub system_prompt: Option<String>,
    pub messages: Vec<ModelMessage>,
    pub tools: Vec<ToolDefinition>,
}
