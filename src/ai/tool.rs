pub mod read_file;

use async_trait::async_trait;
use color_eyre::Result;

#[async_trait]
pub trait Tool: Send + Sync + 'static {
    fn name(&self) -> String;

    fn description(&self) -> Option<String>;

    fn parameters(&self) -> Option<serde_json::Value>;

    async fn execute(&self, args: serde_json::Value) -> color_eyre::Result<String>;
}

pub async fn execute_tool<T: Tool>(tool: T, args: serde_json::Value) -> color_eyre::Result<String> {
    tool.execute(args).await
}
