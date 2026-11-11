use std::{path::PathBuf, time::Duration};

use crate::ai::types::{ModelMessage, ToolCall, Usage};
use crate::session::SessionMeta;

#[derive(Debug)]
pub enum Role {
    User,
    Assistant,
    Error,
}

#[derive(Debug)]
pub enum AgentEvent {
    TurnStarted,
    SessionCleared,
    SessionLoaded {
        meta: SessionMeta,
        messages: Vec<ModelMessage>,
    },
    SessionLoadFailed(String),
    TitleUpdated(String),
    TextDelta(String),
    ReasoningDelta(String),
    ToolStarted(ToolCall),
    ToolResult {
        call_id: String,
        output: String,
        status: ToolStatus,
    },
    UsageUpdated(Usage),
    /// 一次模型流式生成结束后的吞吐统计
    GenerationStats {
        /// 本次生成输出的 token 数
        completion_tokens: u64,
        /// 从发起请求到流结束的耗时
        elapsed: Duration,
    },
    TurnFinished(TurnResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Completed,
    Failed,
    Cancelled,
}
#[derive(Debug)]
pub enum TurnResult {
    Completed,
    Cancelled,
    Failed(String),
}

pub enum UserCommand {
    Submit(String),
    NewSession,
    Resume(PathBuf),
    Cancel,
    Shutdown,
}
