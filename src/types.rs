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
}

#[derive(Debug)]
pub enum AgentEvent {
    Started,
    Delta(ChoiceDelta),
    Done,
    Error(String),
}

pub enum UserCommand {
    Submit(String),
    Shutdown,
}
