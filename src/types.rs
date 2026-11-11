use std::path::PathBuf;

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
