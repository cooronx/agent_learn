use crate::ai::types::{ToolCall, Usage};

#[derive(Debug)]
pub enum Role {
    User,
    Assistant,
    Error,
}

#[derive(Debug)]
pub enum AgentEvent {
    TurnStarted,
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

#[derive(Debug)]
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
    Cancel,
    Shutdown,
}
