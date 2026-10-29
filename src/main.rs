mod agent_loop;
mod ai;
mod app;
mod session;
mod types;
use std::io::stdout;

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
    let client = OpenAIChatCompletionClient::from_env()?;
    let provider: Box<dyn Provider> =
        Box::new(DeepSeekProvider::new(client, "deepseek-v4.1-flash"));
    let (command_tx, command_rx) = mpsc::channel(16);
    let (event_tx, event_rx) = mpsc::channel(100);
    let agent = Agent::new(provider, event_tx, command_rx);
    let terminal = ratatui::init();
    let mouse_result = execute!(stdout(), EnableMouseCapture);
    if let Err(error) = mouse_result {
        ratatui::restore();
        return Err(error.into());
    }
    let task = tokio::spawn(agent.run());
    let result = App::new(command_tx.clone(), event_rx, 128_000)
        .run(terminal)
        .await;
    // App drops its event receiver on exit, releasing any blocked event send.
    let _ = command_tx.send(types::UserCommand::Shutdown).await;
    let mouse_result = execute!(stdout(), DisableMouseCapture);
    ratatui::restore();
    let agent_result = task.await?;
    result?;
    mouse_result?;
    agent_result
}
