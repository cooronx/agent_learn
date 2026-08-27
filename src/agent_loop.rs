use tokio::sync::mpsc;

use crate::{
    ai::ModelSetup, api::openai_compatible::OpenAICompatibleChunk, tool::read_file::ReadFileTool, types::{
        self,
        AgentEvent::{Delta, Done, Error, Started},
        ChoiceDelta::{OutputDelta, ReasoningDelta},
        Message::{self, AgentMessage},
        UserCommand,
    },
};
use async_openai::{
    Client,
    config::OpenAIConfig,
    types::{
        admin::users::User,
        chat::{
            ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
            ChatCompletionRequestSystemMessage, ChatCompletionRequestSystemMessageArgs,
            ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequest,
            CreateChatCompletionRequestArgs,
        },
    },
};
use futures::{SinkExt, StreamExt};

#[derive(Debug)]
pub struct Agent {
    client: async_openai::Client<OpenAIConfig>,
    model_str: String,
    sender: mpsc::Sender<types::Message>,
    receiver: mpsc::Receiver<Message>,
    context: Vec<ChatCompletionRequestMessage>,
}

impl Agent {
    pub fn new(
        setup: ModelSetup,
        sender: mpsc::Sender<Message>,
        receiver: mpsc::Receiver<Message>,
    ) -> Self {
        Self {
            client: setup.client,
            model_str: setup.model,
            sender,
            receiver,
            context: Vec::default(),
        }
    }

    pub fn build_user_propmt(&mut self) -> color_eyre::Result<CreateChatCompletionRequest> {
        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model_str)
            .messages(self.context.clone())
            .tools([ReadFileTool::default().into()])
            .stream(true)
            .build()?;
        Ok(request)
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
        if self.context.is_empty() {
            let prompt = self.build_system_prompt()?;
            let prompt = ChatCompletionRequestSystemMessageArgs::default()
                .content(prompt)
                .build()?;
            self.context.push(prompt.into());
        }
        while let Some(message) = self.receiver.recv().await {
            match message {
                Message::UserMessage(user_command) => match user_command {
                    UserCommand::Submit(msg) => {
                        let ret = async {
                            // 放入上下文中
                            self.context.push(
                                ChatCompletionRequestUserMessageArgs::default()
                                    .content(msg)
                                    .build()?
                                    .into(),
                            );
                            let req = self.build_user_propmt()?;
                            self.send_to_ai_with_stream(req).await
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

    pub async fn send_to_ai_with_stream(
        &mut self,
        request: CreateChatCompletionRequest,
    ) -> color_eyre::Result<()> {
        self.sender.send(Message::AgentMessage(Started)).await?;
        // 这里要改成使用我们自己的回复类型来解析，因为openai api里面没有reasoning content
        let mut stream = self
            .client
            .chat()
            .create_stream_byot::<_, OpenAICompatibleChunk>(request)
            .await?;
        let mut final_output = String::default();
        while let Some(result) = stream.next().await {
            let resp = result?;
            if let Some(choice) = resp.choices.first() {
                // 推理过程
                if let Some(reasoning_content) = &choice.delta.reasoning_content {
                    let reasoning_content =
                        AgentMessage(Delta(ReasoningDelta(reasoning_content.clone())));
                    self.sender.send(reasoning_content).await?;
                }

                if let Some(content) = &choice.delta.base.content {
                    final_output.push_str(content);
                    let output_content = AgentMessage(Delta(OutputDelta(content.clone())));
                    self.sender.send(output_content).await?;
                }
            }
        }
        // 加入上下文
        self.context.push(
            ChatCompletionRequestAssistantMessageArgs::default()
                .content(final_output)
                .build()?
                .into(),
        );
        self.sender.send(Message::AgentMessage(Done)).await?;
        Ok(())
    }

    pub async fn agent_loop() {}
}
