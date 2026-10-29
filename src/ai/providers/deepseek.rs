use async_trait::async_trait;
use color_eyre::eyre::eyre;
use futures::StreamExt;

use crate::ai::{
    api::openai_completions::{
        self, OpenAIChatCompletionRequest, OpenAIChatCompletionStreamChunk,
    },
    client::OpenAIChatCompletionClient,
    providers::{Provider, ProviderStream},
    types::{AssistantDelta, AssistantMessage, Context, Model, ToolCallDelta},
};

pub const PROVIDER: &str = "deepseek";
pub const API: &str = "openai-chat-completion";
pub const DEFAULT_MODEL: &str = "deepseek-v4.1-flash";

pub struct DeepSeekProvider {
    client: OpenAIChatCompletionClient,
    model: Model,
}

impl DeepSeekProvider {
    pub fn new(client: OpenAIChatCompletionClient, model_id: impl Into<String>) -> Self {
        Self {
            client,
            model: Model {
                id: model_id.into(),
                provider: PROVIDER.into(),
                api: API.into(),
            },
        }
    }

    fn build_request(&self, context: &Context, stream: bool) -> OpenAIChatCompletionRequest {
        let mut extra_args = serde_json::Map::new();
        if stream {
            extra_args.insert(
                "stream_options".to_string(),
                serde_json::json!({"include_usage": true}),
            );
        }
        openai_completions::build_request(&self.model, context, stream, extra_args)
    }

    fn convert_chunk(chunk: OpenAIChatCompletionStreamChunk) -> Option<AssistantDelta> {
        let usage = chunk.usage;
        // 有可能存在最后一个chunk只有usage，没有choices的情况（虽然deepseek是没有这个问题的）
        let choice = chunk.choices.into_iter().next();

        let (content, reasoning, tool_calls) = match choice {
            Some(choice) => {
                let delta = choice.delta;
                let reasoning = delta
                    .extra
                    .get("reasoning_content")
                    .and_then(serde_json::Value::as_str)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let tool_calls = delta
                    .tool_calls
                    .unwrap_or_default()
                    .into_iter()
                    .map(|call| {
                        let function = call.function;
                        ToolCallDelta {
                            index: call.index,
                            id: call.id,
                            name: function.as_ref().and_then(|f| f.name.clone()),
                            arguments: function.as_ref().and_then(|f| f.arguments.clone()),
                        }
                    })
                    .collect();
                (delta.content, reasoning, tool_calls)
            }
            None => (None, None, Vec::new()),
        };

        Some(AssistantDelta {
            content,
            reasoning,
            tool_calls,
            usage,
        })
    }
}

#[async_trait]
impl Provider for DeepSeekProvider {
    async fn stream(&self, context: &Context) -> color_eyre::Result<ProviderStream> {
        let request = self.build_request(context, true);
        let stream = self.client.stream(request).await?;

        let stream = stream
            .filter_map(|ret| async move {
                match ret {
                    Ok(chunk) => Self::convert_chunk(chunk).map(Ok),
                    Err(err) => Some(Err(err)),
                }
            })
            .boxed();
        Ok(stream)
    }

    async fn complete(&self, context: &Context) -> color_eyre::Result<AssistantMessage> {
        let request = self.build_request(context, false);
        let response = self.client.complete(request).await?;

        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| eyre!("chat completion response has no choices"))?;

        Ok(choice.message.into())
    }
}
