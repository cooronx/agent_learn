use crate::{
    ai::{
        providers::Provider,
        tool::{
            Tool, bash_command::BashCommandTool, edit::EditTool, list::ListTool,
            read_file::ReadFileTool, write::WriteTool,
        },
        types::*,
    },
    types::{AgentEvent, ToolStatus, TurnResult, UserCommand},
};
use color_eyre::{Result, eyre::eyre};
use futures::StreamExt;
use std::collections::{BTreeMap, HashMap};
use tokio::sync::mpsc;

/// 将合并这些操作包装起来
#[derive(Default)]
struct ResponseAccumulator {
    content: String,
    reasoning: String,
    calls: BTreeMap<usize, ToolCallDelta>,
    finish_reason: Option<String>,
}

impl ResponseAccumulator {
    fn push(&mut self, delta: AssistantDelta) {
        if let Some(s) = delta.content {
            self.content.push_str(&s);
        }
        if let Some(s) = delta.reasoning {
            self.reasoning.push_str(&s);
        }
        if delta.finish_reason.is_some() {
            self.finish_reason = delta.finish_reason;
        }
        for delta in delta.tool_calls {
            let call = self.calls.entry(delta.index).or_insert(ToolCallDelta {
                index: delta.index,
                id: None,
                name: None,
                arguments: None,
            });
            if delta.id.is_some() {
                call.id = delta.id;
            }
            if delta.name.is_some() {
                call.name = delta.name;
            }
            if let Some(s) = delta.arguments {
                call.arguments.get_or_insert_default().push_str(&s);
            }
        }
    }

    fn finish(self) -> Result<AssistantMessage> {
        match self.finish_reason.as_deref() {
            Some("stop") if self.calls.is_empty() => {}
            Some("tool_calls") if !self.calls.is_empty() => {}
            reason => return Err(eyre!("incomplete model response: {reason:?}")),
        }
        let calls = self
            .calls
            .into_values()
            .map(|call| {
                Ok(ToolCall {
                    id: call
                        .id
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| eyre!("missing tool id"))?,
                    name: call
                        .name
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| eyre!("missing tool name"))?,
                    arguments: serde_json::from_str(
                        &call
                            .arguments
                            .ok_or_else(|| eyre!("missing tool arguments"))?,
                    )?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(AssistantMessage {
            content: Some(self.content),
            reasoning: Some(self.reasoning),
            tool_calls: if calls.is_empty() { None } else { Some(calls) },
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum StopReason {
    Cancel,
    Shutdown,
}

async fn wait_for_stop(commands: &mut mpsc::Receiver<UserCommand>) -> StopReason {
    loop {
        match commands.recv().await {
            Some(UserCommand::Cancel) => return StopReason::Cancel,
            Some(UserCommand::Shutdown) | None => return StopReason::Shutdown,
            Some(UserCommand::Submit(_)) => {}
        }
    }
}

pub struct Agent {
    provider: Box<dyn Provider>,
    events: mpsc::Sender<AgentEvent>,
    commands: mpsc::Receiver<UserCommand>,
    context: Context,
    tools: HashMap<String, Box<dyn Tool>>,
    shutdown: bool,
}
impl Agent {
    pub fn new(
        provider: Box<dyn Provider>,
        events: mpsc::Sender<AgentEvent>,
        commands: mpsc::Receiver<UserCommand>,
    ) -> Self {
        Self {
            provider,
            events,
            commands,
            context: Context::default(),
            tools: HashMap::new(),
            shutdown: false,
        }
    }

    fn commit(&mut self, message: ModelMessage) {
        self.context.messages.push(message);
    }

    async fn gen_title(provider: &dyn Provider, first_message: &str) -> color_eyre::Result<String> {
        let system_prompt = "Generate a concise title (max 30 characters) in the same language as the user's message. Reply with the title only, no quotes or punctuation.";

        let context = Context {
            messages: vec![
                ModelMessage::System(SystemMessage {
                    content: system_prompt.to_string(),
                }),
                ModelMessage::User(UserMessage {
                    content: first_message.to_string(),
                }),
            ],
            tools: Vec::new(),
        };

        let reply = provider.complete(&context).await?;

        reply
            .content
            .map(|title| title.trim().to_string())
            .filter(|title| !title.is_empty())
            .ok_or_else(|| eyre!("model returned an empty title"))
    }

    fn build_system_prompt(&self) -> color_eyre::Result<String> {
        let system_prompt = format!(
            r#"You are a coding agent belongs to cooronx, naming cooronx's super easy coding agent.

Current working directory: {}

Use the available tools to inspect, modify, and work with the project.

Guidelines:
- Work within the current working directory.
- Inspect relevant files before making changes.
- Make the smallest changes necessary to complete the task.
- Verify your changes when appropriate.
- Keep your final response concise."#,
            std::env::current_dir()?.display()
        );
        Ok(system_prompt)
    }

    pub async fn run(mut self) -> Result<()> {
        self.commit(ModelMessage::System(SystemMessage {
            content: self.build_system_prompt()?,
        }));
        self.register_tool(ReadFileTool::default());
        self.register_tool(ListTool::default());
        self.register_tool(BashCommandTool::default());
        self.register_tool(WriteTool::default());
        self.register_tool(EditTool::default());
        while let Some(command) = self.commands.recv().await {
            match command {
                UserCommand::Submit(msg) => {
                    self.events.send(AgentEvent::TurnStarted).await?;
                    let outcome = match self.run_turn(msg).await {
                        Ok(outcome) => outcome,
                        Err(err) => TurnResult::Failed(err.to_string()),
                    };
                    if self
                        .events
                        .send(AgentEvent::TurnFinished(outcome))
                        .await
                        .is_err()
                    {
                        return Ok(());
                    }
                    if self.shutdown {
                        break;
                    }
                }
                UserCommand::Cancel => {}
                UserCommand::Shutdown => break,
            }
        }
        Ok(())
    }

    async fn run_turn(&mut self, msg: String) -> Result<TurnResult> {
        // 是否为整个turn的第一条消息
        let first = !self
            .context
            .messages
            .iter()
            .any(|m| matches!(m, ModelMessage::User(_)));
        // 要先判断了再提交
        self.commit(ModelMessage::User(UserMessage {
            content: msg.clone(),
        }));

        if first && !msg.trim().is_empty() {
            tokio::select! {
                biased;
                reason = wait_for_stop(&mut self.commands) => {
                    self.shutdown = reason == StopReason::Shutdown;
                    return Ok(TurnResult::Cancelled);
                }
                title = Self::gen_title(self.provider.as_ref(), &msg) => {
                    if let Ok(title) = title { self.events.send(AgentEvent::TitleUpdated(title)).await?; }
                }
            }
        }
        loop {
            let message = tokio::select! {
                // 同时就绪时，优先处理取消或退出。
                biased;
                reason = wait_for_stop(&mut self.commands) => {
                    self.shutdown = reason == StopReason::Shutdown;
                    return Ok(TurnResult::Cancelled);
                }
                response = Self::generate_response(self.provider.as_ref(), &self.context, &self.events) => response?,
            };
            let calls = message.tool_calls.clone().unwrap_or_default();
            self.commit(ModelMessage::Assistant(message));
            if calls.is_empty() {
                return Ok(TurnResult::Completed);
            }
            let mut cancelled = false;
            for call in calls {
                // 工具调用前先检查一下用户取消没有
                while let Ok(command) = self.commands.try_recv() {
                    match command {
                        UserCommand::Cancel => cancelled = true,
                        UserCommand::Shutdown => {
                            cancelled = true;
                            self.shutdown = true;
                        }
                        UserCommand::Submit(_) => {}
                    }
                }
                let (output, status) = if cancelled {
                    ("未执行：本轮已终止".to_string(), ToolStatus::Cancelled)
                } else {
                    self.events
                        .send(AgentEvent::ToolStarted(call.clone()))
                        .await?;
                    let execution = async {
                        let tool = self
                            .tools
                            .get(&call.name)
                            .ok_or_else(|| eyre!("unknown tool: {}", call.name))?;
                        tool.execute(call.arguments.clone()).await
                    };
                    tokio::pin!(execution);
                    let result = tokio::select! {
                        biased;
                        reason = wait_for_stop(&mut self.commands) => {
                            self.shutdown = reason == StopReason::Shutdown;
                            cancelled = true;
                            execution.await
                        }
                        result = &mut execution => result,
                    };
                    match result {
                        Ok(output) => (output, ToolStatus::Completed),
                        Err(err) => (format!("tool execution failed: {err}"), ToolStatus::Failed),
                    }
                };
                self.commit(ModelMessage::ToolResult(ToolCallResultMessage {
                    tool_call_id: call.id.clone(),
                    content: output.clone(),
                }));
                self.events
                    .send(AgentEvent::ToolResult {
                        call_id: call.id,
                        output,
                        status,
                    })
                    .await?;
            }
            if cancelled {
                return Ok(TurnResult::Cancelled);
            }
        }
    }

    async fn generate_response(
        provider: &dyn Provider,
        context: &Context,
        events: &mpsc::Sender<AgentEvent>,
    ) -> Result<AssistantMessage> {
        let mut stream = provider.stream(context).await?;
        let mut response = ResponseAccumulator::default();
        while let Some(delta) = stream.next().await {
            let delta = delta?;
            if let Some(text) = &delta.content {
                events.send(AgentEvent::TextDelta(text.clone())).await?;
            }
            if let Some(text) = &delta.reasoning {
                events
                    .send(AgentEvent::ReasoningDelta(text.clone()))
                    .await?;
            }
            if let Some(usage) = &delta.usage {
                events.send(AgentEvent::UsageUpdated(usage.clone())).await?;
            }
            response.push(delta);
        }
        response.finish()
    }

    fn register_tool<T: Tool>(&mut self, tool: T) {
        let defin = ToolDefinition {
            name: tool.name(),
            description: tool.description(),
            parameters: tool.parameters(),
        };

        let name = defin.name.clone();

        self.context.tools.push(defin);
        self.tools.insert(name, Box::new(tool));
    }
}
