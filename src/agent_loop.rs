use tokio::sync::mpsc;

use crate::{
    ai::{
        self,
        providers::Provider,
        types::{AssistantMessage, ModelMessage, SystemMessage, UserMessage},
    },
    config::AgentConfig,
    types::{
        self,
        AgentEvent::{Delta, Done, Error, Started},
        ChoiceDelta::{OutputDelta, ReasoningDelta},
        Message::{self, AgentMessage},
        UserCommand,
    },
};
use futures::{SinkExt, StreamExt};

pub struct Agent {
    provider: Box<dyn Provider>,
    config: AgentConfig,
    sender: mpsc::Sender<types::Message>,
    receiver: mpsc::Receiver<Message>,
    context: ai::types::Context,
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
        // 如果是空的，说明刚启动，要加入系统提示词
        if self.context.messages.is_empty() {
            let prompt = self.build_system_prompt()?;
            let prompt = ModelMessage::System(SystemMessage { content: prompt });
            self.context.messages.push(prompt);
        }
        while let Some(message) = self.receiver.recv().await {
            match message {
                Message::UserMessage(user_command) => match user_command {
                    UserCommand::Submit(msg) => {
                        let ret = async {
                            let user_msg = ModelMessage::User(UserMessage { content: msg });
                            self.context.messages.push(user_msg);
                            self.send_to_ai_with_stream().await
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

    pub async fn send_to_ai_with_stream(&mut self) -> color_eyre::Result<()> {
        self.sender.send(Message::AgentMessage(Started)).await?;
        // 这里要改成使用我们自己的回复类型来解析，因为openai api里面没有reasoning content
        let mut stream = self.provider.stream(&self.context).await?;
        let mut reasoning_output = String::default();
        let mut final_output = String::default();
        while let Some(result) = stream.next().await {
            let resp = result?;
            if let Some(reasoning_content) = resp.reasoning {
                reasoning_output.push_str(&reasoning_content);
                let reasoning_content =
                    AgentMessage(Delta(ReasoningDelta(reasoning_content.clone())));
                self.sender.send(reasoning_content).await?;
            }
            if let Some(content) = resp.content {
                final_output.push_str(&content);
                let output_content = AgentMessage(Delta(OutputDelta(content.clone())));
                self.sender.send(output_content).await?;
            }
        }
        // 加入上下文
        let assistant_msg = ModelMessage::Assistant(AssistantMessage {
            content: Some(final_output),
            reasoning: Some(reasoning_output),
            tool_calls: None,
        });
        self.context.messages.push(assistant_msg);
        self.sender.send(Message::AgentMessage(Done)).await?;
        Ok(())
    }
}
