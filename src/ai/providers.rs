use std::pin::Pin;

use async_trait::async_trait;
use futures::Stream;

use crate::ai::types::{AssistantDelta, Context};

pub mod deepseek;

pub type ProviderStream = Pin<Box<dyn Stream<Item = color_eyre::Result<AssistantDelta>> + Send>>;

// 所有的provider都应该实现这个东西
#[async_trait]
pub trait Provider: Send + Sync {
    async fn stream(&self, context: &Context) -> color_eyre::Result<ProviderStream>;
}
