use serde::{Deserialize, Serialize};

use crate::ai::types::{
    AssistantMessage, Context, Model, ModelMessage, Role, SystemMessage, ToolCall,
    ToolCallResultMessage, ToolDefinition, UserMessage,
};

type ExtraMapType = serde_json::Map<String, serde_json::Value>;

// ============== Request ===============

#[derive(Debug, Serialize)]
pub struct OpenAIChatCompletionRequest {
    pub model: String,
    pub messages: Vec<OpenAIChatCompletionMessage>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<OpenAIFunctionToolDefinition>,

    pub stream: bool,

    #[serde(flatten)]
    pub extra: ExtraMapType,
}

// ============ Message ================

#[derive(Debug, Deserialize, Serialize)]
pub struct OpenAIChatCompletionMessage {
    pub role: Role,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenAIToolCall>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,

    #[serde(flatten)]
    pub extra: ExtraMapType,
}

impl From<&SystemMessage> for OpenAIChatCompletionMessage {
    fn from(value: &SystemMessage) -> Self {
        Self {
            role: Role::System,
            content: Some(value.content.clone()),
            tool_calls: None,
            tool_call_id: None,
            extra: serde_json::Map::new(),
        }
    }
}

impl From<&UserMessage> for OpenAIChatCompletionMessage {
    fn from(value: &UserMessage) -> Self {
        Self {
            role: Role::User,
            content: Some(value.content.clone()),
            tool_calls: None,
            tool_call_id: None,
            extra: serde_json::Map::new(),
        }
    }
}

impl From<&AssistantMessage> for OpenAIChatCompletionMessage {
    fn from(value: &AssistantMessage) -> Self {
        Self {
            role: Role::Assistant,
            content: value.content.clone(),
            tool_calls: Some(value.tool_calls.iter().map(OpenAIToolCall::from).collect()),
            tool_call_id: None,
            extra: serde_json::Map::new(),
        }
    }
}

impl From<&ToolCallResultMessage> for OpenAIChatCompletionMessage {
    fn from(value: &ToolCallResultMessage) -> Self {
        Self {
            role: Role::Tool,
            content: Some(value.content.clone()),
            tool_calls: None,
            tool_call_id: Some(value.tool_call_id.clone()),
            extra: serde_json::Map::new(),
        }
    }
}

impl From<&ModelMessage> for OpenAIChatCompletionMessage {
    fn from(value: &ModelMessage) -> Self {
        match value {
            ModelMessage::User(user_message) => Self::from(user_message),
            ModelMessage::Assistant(assistant_message) => Self::from(assistant_message),
            ModelMessage::ToolResult(tool_call_result_message) => {
                Self::from(tool_call_result_message)
            }
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
#[derive(Debug, Serialize)]
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

#[derive(Debug, Deserialize, Serialize)]
pub struct OpenAIToolCall {
    pub id: String,

    #[serde(rename = "type")]
    pub kind: String,

    pub function: OpenAIFunctionCall,
}

#[derive(Debug, Deserialize, Serialize)]
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
            },
        }
    }
}

// ======= response =======
#[derive(Debug, Deserialize)]
pub struct OpenAIChatCompletionResponse {
    pub choices: Vec<OpenAIChatCompletionChoice>,

    // 暂时用不到的字段直接保留，后面用到的时候再来实现吧
    #[serde(flatten)]
    pub extra: ExtraMapType,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIChatCompletionChoice {
    pub index: usize,
    pub message: OpenAIChatCompletionMessage,
    pub finish_reason: Option<String>,

    // 暂时用不到的字段直接保留，后面用到的时候再来实现吧
    #[serde(flatten)]
    pub extra: ExtraMapType,
}

// ======= method =======
pub fn build_request(
    model: &Model,
    context: &Context,
    stream: bool,
    extra: serde_json::Map<String, serde_json::Value>,
) -> OpenAIChatCompletionRequest {
    let mut messages = Vec::new();

    messages.extend(
        context
            .messages
            .iter()
            .map(OpenAIChatCompletionMessage::from),
    );

    let tools = context
        .tools
        .iter()
        .map(OpenAIFunctionToolDefinition::from)
        .collect();

    OpenAIChatCompletionRequest {
        model: model.id.clone(),
        messages,
        tools,
        stream,
        extra,
    }
}

// ====== stream response =======

#[derive(Debug, Deserialize)]
pub struct OpenAIChatCompletionStreamChunk {
    pub choices: Vec<OpenAIChatCompletionStreamChoice>,

    #[serde(flatten)]
    pub extra: ExtraMapType,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIChatCompletionStreamChoice {
    pub index: usize,
    pub delta: OpenAIChatCompletionDelta,
    pub finish_reason: Option<String>,

    #[serde(flatten)]
    pub extra: ExtraMapType,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIChatCompletionDelta {
    pub content: Option<String>,
    pub role: Option<Role>,
    pub tool_calls: Option<Vec<OpenAIToolCallDelta>>,

    #[serde(flatten)]
    pub extra: ExtraMapType,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIToolCallDelta {
    pub index: usize,
    pub id: Option<String>,
    pub function: Option<OpenAIFunctionCallDelta>,

    #[serde(rename = "type")]
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIFunctionCallDelta {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[cfg(test)]
mod test {
    use futures::StreamExt;

use crate::ai::{ModelSetup, client::OpenAIChatCompletionClient};

    use super::*;

    #[tokio::test]
    async fn send_request() {
        dotenvy::dotenv().ok();
        let client = OpenAIChatCompletionClient::from_env().unwrap();
        let model = Model {
            id: "deepseek-v4-flash".to_string(),
            provider: "deepseek".to_string(),
            api: "todo!()".to_string(),
        };
        let context = Context {
            system_prompt: None,
            messages: vec![ModelMessage::User(UserMessage {
                content: "你好啊".to_string(),
            })],
            tools: Vec::new(),
        };

        let req = build_request(&model, &context, true, serde_json::Map::new());
        let mut stream = client.stream(req).await.unwrap();
        while let Some(chunk) = stream.next().await {
            
            let chunk = chunk.unwrap();
            println!("{:?}",chunk.choices[0]);
        }
    }
}
