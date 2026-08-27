use color_eyre::eyre::eyre;
use reqwest::Client;

use crate::ai::api::openai_completions::{
    OpenAIChatCompletionRequest, OpenAIChatCompletionResponse,
};

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
}
