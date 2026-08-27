
use serde::{Serialize,Deserialize};

use crate::ai::types::{AssistantMessage, Context, Model, ModelMessage, Role, SystemMessage, ToolCall, ToolCallResultMessage, ToolDefinition, UserMessage};


// ============== Request ===============

#[derive(Debug,Serialize)]
pub struct OpenAIChatCompletionRequest {
    pub model: String,
    pub messages: Vec<OpenAIMessage>,

    #[serde(default,skip_serializing_if = "Vec::is_empty")]
    pub tools : Vec<OpenAIFunctionToolDefinition>,

    pub stream: bool,

    #[serde(flatten)]
    pub extra: serde_json::Map<String,serde_json::Value>,
}


// ============ Message ================

#[derive(Debug,Deserialize,Serialize)]
pub struct OpenAIMessage {
    pub role: Role,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<OpenAIToolCall>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,

    #[serde(flatten)]
    pub extra: serde_json::Map<String,serde_json::Value>,
}

impl From<&SystemMessage> for OpenAIMessage {
    fn from(value: &SystemMessage) -> Self {
        Self {
            role: Role::System,
            content: Some(value.content.clone()),
            tool_calls: Vec::default(),
            tool_call_id: None,
            extra: serde_json::Map::new(),
        }
    }
}

impl From<&UserMessage> for OpenAIMessage {
    fn from(value: &UserMessage) -> Self {
        Self {
            role: Role::User,
            content: Some(value.content.clone()),
            tool_calls: Vec::default(),
            tool_call_id: None,
            extra: serde_json::Map::new(),
        }
    }
}

impl From<&AssistantMessage> for OpenAIMessage {
    fn from(value: &AssistantMessage) -> Self {
        Self {
            role: Role::Assistant,
            content: value.content.clone(),
            tool_calls: value.tool_calls.iter().map(OpenAIToolCall::from).collect(),
            tool_call_id: None,
            extra: serde_json::Map::new(),
        }
    }
}

impl From<&ToolCallResultMessage> for OpenAIMessage {
    fn from(value: &ToolCallResultMessage) -> Self {
        Self {
            role: Role::Tool,
            content: Some(value.content.clone()),
            tool_calls: Vec::default(),
            tool_call_id: Some(value.tool_call_id.clone()),
            extra: serde_json::Map::new(),
        }
    }
}

impl From<&ModelMessage> for OpenAIMessage {
    fn from(value: &ModelMessage) -> Self {
        match value {
            ModelMessage::User(user_message) => Self::from(user_message),
            ModelMessage::Assistant(assistant_message) => Self::from(assistant_message),
            ModelMessage::ToolResult(tool_call_result_message) => Self::from(tool_call_result_message),
            ModelMessage::System(system_message) => Self::from(system_message),
        }
    }
}


// ============== Tool ==================

// 需要额外包装一层，因为openai的tool序列化要求如下形式
// {
//      "type": "function",
//      "function": {
//         "name": "read_file",
//         "description": "读取文件",
//         "parameters": {}
//      }
// }
// 
#[derive(Debug,Serialize)]
pub struct OpenAIFunctionToolDefinition {

    // 这里不能直接写type，因为type是rust的关键字，好家伙
    #[serde(rename = "type")]
    pub kind: String,

    pub function: ToolDefinition,
}

impl From<&ToolDefinition> for OpenAIFunctionToolDefinition {
    fn from(value: &ToolDefinition) -> Self {
        Self {
            kind: "function".to_string(),
            function: value.clone(),
        }
    }
}

// ======== ToolCall =======

// 需要额外包装一层
// {
//       "id": "call_abc123",
//       "type": "function",
//       "function": {
//         "name": "get_weather",
//         "arguments": "{\"city\":\"Beijing\",\"unit\":\"celsius\"}"
//       }
// }

#[derive(Debug,Deserialize,Serialize)]
pub struct OpenAIToolCall {
    pub id: String,

    #[serde(rename = "type")]
    pub kind: String,

    pub function: OpenAIFunctionCall,
}

#[derive(Debug,Deserialize,Serialize)]
pub struct OpenAIFunctionCall {
    pub name: String,
    
    // openai 要求这里是字符串
    pub arguments: String,
}

impl From<&ToolCall> for OpenAIToolCall {
    fn from(value: &ToolCall) -> Self {
        Self { 
            id: value.id.clone(),
            kind: "function".to_string(),
            function: OpenAIFunctionCall {
                name: value.name.clone(),
                arguments: value.arguments.to_string(),
            }
        }
    }
}


// ======= method =======
pub fn build_request(model: &Model, context: &Context, stream: bool, extra: serde_json::Map<String,serde_json::Value>) -> OpenAIChatCompletionRequest {

    let mut messages = Vec::new();

    messages.extend(context.messages.iter().map(OpenAIMessage::from));

    let tools = context.tools.iter().map(OpenAIFunctionToolDefinition::from).collect();

    OpenAIChatCompletionRequest { model: model.id.clone(), messages, tools, stream, extra }
}

// #[cfg(test)]
// mod test {
//     use crate::ai::ModelSetup;

// use super::*;

//     #[test]
//     fn send_request() {
//         dotenvy::dotenv().ok();
//         let model_setup = ModelSetup::from_env().unwrap();
//         // let req = build_request(model_setup, context, stream, extra)
//     }

// }