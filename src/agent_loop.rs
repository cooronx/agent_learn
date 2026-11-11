use crate::{
    ai::{
        providers::Provider,
        tool::{
            Tool, bash_command::BashCommandTool, edit::EditTool, list::ListTool,
            read_file::ReadFileTool, write::WriteTool,
        },
        types::*,
    },
    session::Session,
    types::{AgentEvent, ToolStatus, TurnResult, UserCommand},
};
use color_eyre::{Result, eyre::eyre};
use futures::StreamExt;
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    time::Duration,
};
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
            Some(UserCommand::Submit(_) | UserCommand::NewSession | UserCommand::Resume(_)) => {}
        }
    }
}

pub struct Agent {
    provider: Box<dyn Provider>,
    events: mpsc::Sender<AgentEvent>,
    commands: mpsc::Receiver<UserCommand>,
    context: Context,
    tools: HashMap<String, Box<dyn Tool>>,
    session: Option<Session>,
    session_dir: PathBuf,
    model: String,
    shutdown: bool,
}
impl Agent {
    pub fn new(
        provider: Box<dyn Provider>,
        events: mpsc::Sender<AgentEvent>,
        commands: mpsc::Receiver<UserCommand>,
        session_dir: PathBuf,
        model: String,
    ) -> Self {
        Self {
            provider,
            events,
            commands,
            context: Context::default(),
            tools: HashMap::new(),
            session: None,
            session_dir,
            model,
            shutdown: false,
        }
    }

    /// 落地历史消息
    fn commit(&mut self, message: ModelMessage) -> Result<()> {
        if !matches!(message, ModelMessage::System(_)) {
            if let Some(session) = &mut self.session {
                session.append_message(&message)?;
            }
        }
        self.context.messages.push(message);
        Ok(())
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

        let title = reply.content.unwrap_or_default();
        let title = title.lines().next().unwrap_or("").trim();

        if title.is_empty() || title.chars().count() > 60 {
            return Err(eyre!("model returned an invalid title"));
        }
        Ok(title.to_string())
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
- When the user asks for a code change, actually make the change with the available tools in this turn. Do not stop after describing a plan.
- Keep planning and internal reasoning out of the final answer; the final answer should only summarize what was actually done.
- Verify your changes when appropriate.
- Keep your final response concise."#,
            std::env::current_dir()?.display()
        );
        Ok(system_prompt)
    }

    fn reset_context(&mut self) -> Result<()> {
        self.context.messages.clear();
        self.context
            .messages
            .push(ModelMessage::System(SystemMessage {
                content: self.build_system_prompt()?,
            }));
        Ok(())
    }

    async fn start_new_session(&mut self) -> Result<()> {
        self.session = None;
        self.reset_context()?;
        self.events.send(AgentEvent::SessionCleared).await?;
        Ok(())
    }

    async fn resume_session(&mut self, path: PathBuf) -> Result<()> {
        match Session::open(&path) {
            Ok((session, messages)) => {
                let meta = session.meta().clone();
                self.session = Some(session);
                self.reset_context()?;
                self.context.messages.extend(messages.clone());
                self.events
                    .send(AgentEvent::SessionLoaded { meta, messages })
                    .await?;
            }
            Err(error) => {
                self.events
                    .send(AgentEvent::SessionLoadFailed(error.to_string()))
                    .await?;
            }
        }
        Ok(())
    }

    pub async fn run(mut self) -> Result<()> {
        self.reset_context()?;
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
                UserCommand::NewSession => {
                    self.start_new_session().await?;
                }
                UserCommand::Resume(path) => {
                    self.resume_session(path).await?;
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

        // 只有用户开始了第一轮才会创建会话文件
        if first && self.session.is_none() && !msg.trim().is_empty() {
            let fallback_title: String = msg
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .chars()
                .take(60)
                .collect();
            let fallback_title = if fallback_title.is_empty() {
                "Untitled".to_string()
            } else {
                fallback_title
            };
            let cwd = std::env::current_dir()?.display().to_string();

            self.session = Some(Session::create(
                &self.session_dir,
                &cwd,
                &fallback_title,
                &self.model,
            )?);
        }

        // 要先判断了再提交
        self.commit(ModelMessage::User(UserMessage {
            content: msg.clone(),
        }))?;

        if first && !msg.trim().is_empty() {
            tokio::select! {
                biased;
                reason = wait_for_stop(&mut self.commands) => {
                    self.shutdown = reason == StopReason::Shutdown;
                    return Ok(TurnResult::Cancelled);
                }
                title = tokio::time::timeout(
                    Duration::from_secs(5),
                    Self::gen_title(self.provider.as_ref(), &msg),
                ) => {
                    if let Ok(Ok(title)) = title {
                        // 标题落盘失败不影响本轮，UI 仍会显示生成的标题
                        if let Some(session) = &mut self.session {
                            let _ = session.set_title(&title);
                        }
                        self.events.send(AgentEvent::TitleUpdated(title)).await?;
                    }
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
            self.commit(ModelMessage::Assistant(message))?;
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
                        UserCommand::Submit(_)
                        | UserCommand::NewSession
                        | UserCommand::Resume(_) => {}
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
                }))?;
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
        let started_at = std::time::Instant::now();
        let mut stream = provider.stream(context).await?;
        let mut response = ResponseAccumulator::default();
        let mut completion_tokens = 0;
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
                completion_tokens = usage.completion_tokens;
                events.send(AgentEvent::UsageUpdated(usage.clone())).await?;
            }
            response.push(delta);
        }
        
        if completion_tokens > 0 {
            events
                .send(AgentEvent::GenerationStats {
                    completion_tokens,
                    elapsed: started_at.elapsed(),
                })
                .await?;
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
