use derive_builder::Builder;

#[derive(Debug, Builder, Default)]
#[builder(setter(into))]
pub struct AgentConfig {
    model: String,
}

impl AgentConfig {
    pub fn model(&self) -> &str {
        &self.model
    }
}
