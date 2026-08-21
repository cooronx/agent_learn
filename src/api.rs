use async_openai::{Client, config::OpenAIConfig};

use color_eyre::{Result, eyre::Context};

pub mod openai_compatible;

pub struct ModelSetup {
    pub client: async_openai::Client<OpenAIConfig>,
    pub model: String,
}

impl ModelSetup {
    pub fn from_env() -> Result<Self> {
        let base_url = std::env::var("OPENAI_BASE_URL").wrap_err("未找到base_url")?;
        let key = std::env::var("OPENAI_API_KEY").wrap_err("未找到api_key")?;
        let config = OpenAIConfig::default()
            .with_api_base(base_url)
            .with_api_key(key);
        let client = Client::with_config(config);

        Ok(Self {
            client,
            model: String::from("deepseek-v4-flash"),
        })
    }
}
