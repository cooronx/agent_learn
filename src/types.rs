#[derive(Debug)]
pub enum Role {
    User,
    Assistant,
    Error,
}

pub enum Message {
    AgentMessage(AgentEvent),
    UserMessage(UserCommand),
}

#[derive(Debug)]
pub enum ChoiceDelta {
    OutputDelta(String),
    ReasoningDelta(String),
    ToolCallContent(String),
}

#[derive(Debug)]
pub enum AgentEvent {
    Started,
    Delta(ChoiceDelta),
    Done,
    Error(String),
    /// 本次请求的 token 用量，用来在右侧栏展示上下文占用情况
    Usage {
        /// 输入 token 数，也就是当前上下文占用的 token
        prompt_tokens: u64,
        /// 输入 + 输出累积的 token 数
        total_tokens: u64,
    },
}

pub enum UserCommand {
    Submit(String),
    Shutdown,
}
