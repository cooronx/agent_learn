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
pub enum AgentEvent {
    Started,
    Delta(String),
    Done,
    Error(String),
}

pub enum UserCommand {
    Submit(String),
    Shutdown,
}
