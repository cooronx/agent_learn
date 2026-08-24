use tokio::sync::mpsc;

use crate::{
    api::{ModelSetup, openai_compatible::OpenAICompatibleChunk},
    types::{
        self,
        AgentEvent::{self, Delta, Done, Error, Started},
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
        }
    }

    pub fn build_user_propmt(
        &mut self,
        prompt: String,
    ) -> color_eyre::Result<CreateChatCompletionRequest> {
        let request = CreateChatCompletionRequestArgs::default()
            .model(&self.model_str)
            .messages([ChatCompletionRequestUserMessageArgs::default()
                .content(prompt)
                .build()?
                .into()])
            .stream(true)
            .build()?;
        Ok(request)
    }

    pub async fn run(mut self) -> color_eyre::Result<()> {
        while let Some(message) = self.receiver.recv().await {
            match message {
                Message::UserMessage(user_command) => match user_command {
                    UserCommand::Submit(msg) => {
                        let ret = async {
                            let req = self.build_user_propmt(msg)?;
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
        while let Some(result) = stream.next().await {
            let resp = result?;
            if let Some(choice) = resp.choices.first() {
                // 新加入的推理过程
                if let Some(reasoning_content) = &choice.delta.reasoning_content {
                    let reasoning_content =
                        AgentMessage(Delta(ReasoningDelta(reasoning_content.clone())));
                    self.sender.send(reasoning_content).await?;
                }

                if let Some(content) = &choice.delta.base.content {
                    let output_content = AgentMessage(Delta(OutputDelta(content.clone())));
                    self.sender.send(output_content).await?;
                }
            }
        }
        self.sender.send(Message::AgentMessage(Done)).await?;
        Ok(())
    }
}
