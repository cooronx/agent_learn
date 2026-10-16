mod agent_loop;
mod ai;
mod app;
mod types;
use std::{io::stdout, sync::{Arc, atomic::AtomicBool}};

use crate::{
    agent_loop::Agent,
    ai::{
        client::OpenAIChatCompletionClient,
        providers::{Provider, deepseek::DeepSeekProvider},
    },
    app::App,
};

use color_eyre::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    color_eyre::install()?;
    let terminal = ratatui::init();
    // 激活鼠标捕捉
    execute!(stdout(), EnableMouseCapture)?;
    // 这个是用来用户和agent之间交互的通道
    let (user_msg_sender, user_msg_recv) = mpsc::channel(100);
    // 这个是agent和tui界面之间交互的通道
    let (agent_msg_sender, agent_msg_recv) = mpsc::channel(100);

    let client = OpenAIChatCompletionClient::from_env()?;
    let provider: Box<dyn Provider> = Box::new(DeepSeekProvider::new(client, "deepseek-v4.1-flash"));

    let is_cancelled = Arc::new(AtomicBool::new(false));

    let agent = Agent::new(provider, agent_msg_sender.clone(), user_msg_recv,is_cancelled.clone());
    let task = tokio::spawn(agent.run());
    App::new(user_msg_sender.clone(), agent_msg_recv,is_cancelled.clone())
        .run(terminal)
        .await?;
    task.abort();
    execute!(stdout(), DisableMouseCapture)?;
    ratatui::restore();

    Ok(())
}
