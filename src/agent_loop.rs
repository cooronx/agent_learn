use std::collections::HashMap;

use color_eyre::eyre::{self, eyre};
use serde::de;
use tokio::sync::mpsc;

use crate::{
    ai::{
        self, providers::Provider, tool::{Tool, list::ListTool, read_file::ReadFileTool}, types::{
            AssistantMessage, ModelMessage, SystemMessage, ToolCall, ToolCallDelta,
            ToolCallResultMessage, ToolDefinition, UserMessage,
        },
    }, config::AgentConfig, types::{
        self, AgentEvent::{Delta, Done, Error, Started}, ChoiceDelta::{OutputDelta, ReasoningDelta, ToolCallContent}, Message::{self, AgentMessage}, UserCommand,
    },
};
use futures::{SinkExt, StreamExt};

pub struct Agent {
    provider: Box<dyn Provider>,
    config: AgentConfig,
    sender: mpsc::Sender<types::Message>,
    receiver: mpsc::Receiver<Message>,
    context: ai::types::Context,
    tools: HashMap<String, Box<dyn Tool>>,
}

impl Agent {
    pub fn new(
        provider: Box<dyn Provider>,
        config: AgentConfig,
        sender: mpsc::Sender<Message>,
        receiver: mpsc::Receiver<Message>,
    ) -> Self {
        Self {
            provider,
            config,
            sender,
            receiver,
            context: ai::types::Context::default(),
            tools: HashMap::new(),
        }
    }

    fn build_system_prompt(&self) -> color_eyre::Result<String> {
        let system_prompt = format!(
            r#"You are a coding agent belongs to cooronx, naming cooronx的超级简单coding agent.

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

    pub async fn run(mut self) -> color_eyre::Result<()> {
        // 如果是空的，说明刚启动，要加入系统提示词和我们内置的那些工具
        if self.context.messages.is_empty() {
            // 系统提示词
            let prompt = self.build_system_prompt()?;
            let prompt = ModelMessage::System(SystemMessage { content: prompt });
            self.context.messages.push(prompt);
            // 工具定义
            self.register_tool(ReadFileTool::default());
            self.register_tool(ListTool::default());
        }
        while let Some(message) = self.receiver.recv().await {
            match message {
                Message::UserMessage(user_command) => match user_command {
                    UserCommand::Submit(msg) => {
                        let ret = async {
                            let user_msg = ModelMessage::User(UserMessage { content: msg });
                            self.context.messages.push(user_msg);
                            // 核心的agent循环
                            self.agent_loop().await
                        }
                        .await;
                        // 错误信息也一并发过去展示
                        if let Err(err) = ret {
                            self.sender
                                .send(AgentMessage(Error(err.to_string())))
                                .await?;
                        }
                    }
                    UserCommand::Shutdown => {
                        return Ok(());
                    }
                },
                Message::AgentMessage(_) => {}
            }
        }
        Ok(())
    }

    pub async fn agent_loop(&mut self) -> color_eyre::Result<()> {
        self.sender.send(Message::AgentMessage(Started)).await?;

        loop {
            let mut stream = self.provider.stream(&self.context).await?;
            let mut reasoning_output = String::default();
            let mut final_output = String::default();
            let mut pending_calls: Vec<ToolCallDelta> = Vec::new();
            while let Some(result) = stream.next().await {
                let resp = result?;
                // 思维链
                if let Some(reasoning_content) = resp.reasoning {
                    reasoning_output.push_str(&reasoning_content);
                    let reasoning_content =
                        AgentMessage(Delta(ReasoningDelta(reasoning_content.clone())));
                    self.sender.send(reasoning_content).await?;
                }
                // 回复
                if let Some(content) = resp.content {
                    final_output.push_str(&content);
                    let output_content = AgentMessage(Delta(OutputDelta(content.clone())));
                    self.sender.send(output_content).await?;
                }
                // 工具调用(按照index流式拼接起来)
                for delta in resp.tool_calls {
                    // chunk1: {"tool_calls":[{"index":0,"id":"call_abc123","type":"function","function":{"name":"read_file","arguments":""}}]}
                    //         ->
                    // chunk2: {"tool_calls":[{"index":0,"function":{"arguments":"{"path":""}}]}
                    //         ->
                    // chunk3: {"tool_calls":[{"index":0,"function":{"arguments":"src/main.rs"}}]}
                    //         ->
                    // chunk4: {"tool_calls":[{"index":0,"function":{"arguments":""}"}}]}
                    while pending_calls.len() <= delta.index {
                        // 如果没有，那就先创建
                        pending_calls.push(ToolCallDelta {
                            index: delta.index,
                            id: delta.id.clone(),
                            name: delta.name.clone(),
                            arguments: None,
                        });
                    }
                    let current_delta = &mut pending_calls[delta.index];


                    // 不能默认第一个sse chunk是有name和id的
                    if delta.name.is_some() {
                        current_delta.name = delta.name;
                    }

                    if delta.id.is_some() {
                        current_delta.id = delta.id;
                    }

                    if let Some(frag) = delta.arguments {
                        current_delta
                            .arguments
                            .get_or_insert_default()
                            .push_str(&frag);
                    }
                }
            }

            // 拼接tool_call
            let tool_calls: Vec<ToolCall> = pending_calls
                .into_iter()
                .map(|delta| {
                    let arguments = delta
                        .arguments
                        .ok_or_else(|| eyre!("missing tool arguments"))?;
                    let name = delta.name.ok_or_else(|| eyre!("missing tool name"))?;
                    let id = delta.id.ok_or_else(|| eyre!("missing tool id"))?;
                    Ok(ToolCall {
                        arguments: serde_json::from_str(&arguments)?,
                        name,
                        id,
                    })
                })
                .collect::<color_eyre::Result<_>>()?;

            // 加入上下文
            let assistant_msg = ModelMessage::Assistant(AssistantMessage {
                content: Some(final_output),
                reasoning: Some(reasoning_output),
                tool_calls: if tool_calls.is_empty() {None} else {Some(tool_calls.clone())},
            });

            self.context.messages.push(assistant_msg);

            if tool_calls.is_empty() {
                break;
            }


            // 执行toolcall
            for tool_call in tool_calls {
                // 发送工具调用给tui界面
                let tool_call_content = AgentMessage(Delta(ToolCallContent(format!("{} {}",tool_call.name,tool_call.arguments))));
                self.sender.send(tool_call_content).await?;
                let content = match self.execute_tool_call(&tool_call).await {
                    Ok(output) => output,
                    Err(err) => {
                        format!("tool execution failed: {err}")
                    }
                };
                self.context
                    .messages
                    .push(ModelMessage::ToolResult(ToolCallResultMessage {
                        tool_call_id: tool_call.id,
                        content,
                    }));
            }
            
        }

        self.sender.send(Message::AgentMessage(Done)).await?;
        Ok(())
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

    async fn execute_tool_call(&self, tool_call: &ToolCall) -> color_eyre::Result<String> {
        let tool = self
            .tools
            .get(&tool_call.name)
            .ok_or_else(|| eyre!("unknown tool: {}", tool_call.name))?;

        tool.execute(tool_call.arguments.clone()).await
    }
}
