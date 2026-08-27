use std::pin::Pin;

use color_eyre::eyre::eyre;
use eventsource_stream::Eventsource;
use futures::{Stream, StreamExt, future::ready};
use reqwest::Client;

use crate::ai::api::openai_completions::{
    OpenAIChatCompletionRequest, OpenAIChatCompletionResponse, OpenAIChatCompletionStreamChunk,
};

pub type OpenAIChatCompletionStream =
    Pin<Box<dyn Stream<Item = color_eyre::Result<OpenAIChatCompletionStreamChunk>> + Send>>;

#[derive(Debug)]
pub struct OpenAIChatCompletionClient {
    pub client: Client,
    pub base_url: String,
    pub api_key: String,
}

impl OpenAIChatCompletionClient {
    pub fn from_env() -> color_eyre::Result<Self> {
        let base_url = std::env::var("OPENAI_BASE_URL")?;
        let api_key = std::env::var("OPENAI_API_KEY")?;
        let oai_client = Self {
            client: Client::new(),
            base_url,
            api_key,
        };
        Ok(oai_client)
    }

    pub async fn complete(
        &self,
        request: OpenAIChatCompletionRequest,
    ) -> color_eyre::Result<OpenAIChatCompletionResponse> {
        let req_url = std::format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        let resp = self
            .client
            .post(req_url)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await?;

        let status = resp.status();

        // 排除非200的
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();

            return Err(eyre!(
                "chat completion request failed: HTTP {} : {}",
                status,
                body
            ));
        }

        let resp = resp.json::<OpenAIChatCompletionResponse>().await?;

        Ok(resp)
    }

    pub async fn stream(
        &self,
        request: OpenAIChatCompletionRequest,
    ) -> color_eyre::Result<OpenAIChatCompletionStream> {
        if !request.stream {
            return Err(eyre!("sse need field stream set to true"));
        }

        let req_url = std::format!("{}/chat/completions", self.base_url.trim_end_matches('/'));

        let response = self
            .client
            .post(req_url)
            .bearer_auth(&self.api_key)
            .json(&request)
            .send()
            .await?;

        let status = response.status();

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();

            return Err(eyre!(
                "chat completion request failed: HTTP {}: {}",
                status,
                body
            ));
        }

        let stream = response
            .bytes_stream()
            .eventsource()
            .take_while(|event|{
                ready(match event {
                    Ok(event) => event.data.trim() != "[DONE]",
                    Err(_) => true,
                })
            })
            .filter_map(|event| async move {
                match event {
                    Ok(event) if event.data.trim().is_empty() => None,
                    Ok(event) => {
                        let ret = serde_json::from_str::<OpenAIChatCompletionStreamChunk>(&event.data)
                            .map_err(|error| {
                                eyre!("failed to deserialize chunk: {}, data: {}",error,event.data)
                            });
                        Some(ret)
                    }
                    Err(error) => {
                        Some(Err(eyre!("failed to read SSE event: {}",error)))
                    },
                }
            })
            .boxed();

        Ok(stream)
    }
}
