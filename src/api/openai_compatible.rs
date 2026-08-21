//! 原始的openai chat completion api没有提供思考字段，所以得搞一个openai兼容的接口


use std::collections::HashMap;

use async_openai::types::chat::{ChatChoiceLogprobs, ChatCompletionStreamResponseDelta, FinishReason};
use serde_json::Value;



#[derive(Debug,serde::Deserialize)]
pub struct OpenAICompatibleDelta {
    #[serde(flatten)]
    pub base: ChatCompletionStreamResponseDelta,

    // deepseek的推理字段
    pub reasoning_content: Option<String>,
}

#[derive(Debug,serde::Deserialize)] 
pub struct OpenAICompatibleChoice {
    pub index: u32,
    pub delta: OpenAICompatibleDelta,
    pub finish_reason: Option<FinishReason>,
    pub logprobs: Option<ChatChoiceLogprobs>,
}

#[derive(Debug,serde::Deserialize)]
pub struct OpenAICompatibleChunk {
    pub choices: Vec<OpenAICompatibleChoice>,

    #[serde(flatten)]
    pub extra: HashMap<String,Value> 
}