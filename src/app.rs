use async_openai::types::admin::users::User;
use color_eyre::Result;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate},
};
use futures::{FutureExt, StreamExt};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::{Buffer, CellDiffOption},
    layout::{Constraint, Layout, Rect},
    style::{Style, Stylize},
    text::Line,
    widgets::{Block, Paragraph},
};
use ratatui_textarea::{TextArea, WrapMode};
use tokio::sync::mpsc;

use crate::types::{
    Message, Role::{self, Assistant, User as TuiUser}, UserCommand,
};

#[derive(Debug)]
struct DisplayMessage {
    role: Role,
    content: String,
}

#[derive(Debug)]
pub struct App<'a> {
    running: bool,
    message_recv: mpsc::Receiver<Message>,
    message_sender: mpsc::Sender<Message>,
    display_message: Vec<DisplayMessage>,
    current_assitant_index: Option<usize>,
    event_stream: EventStream,
    inputs: TextArea<'a>,
}

impl App<'_> {
    pub fn new(sender: mpsc::Sender<Message>, recv: mpsc::Receiver<Message>) -> Self {
        let mut input = TextArea::default();
        input.set_block(Block::bordered().title("输入框"));
        Self {
            running: false,
            message_recv: recv,
            message_sender: sender,
            display_message: Vec::default(),
            current_assitant_index: None,
            event_stream: EventStream::default(),
            inputs: input,
        }
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        self.running = true;
        while self.running {
            terminal.draw(|frame| self.draw(frame))?;
            tokio::select! {
                event = self.event_stream.next() => {
                  if let Some(Ok(Event::Key(key))) = event
                      && key.kind == KeyEventKind::Press
                  {
                      self.on_key_event(key).await?;
                  }
              }

                message = self.message_recv.recv() => {
                        if let Some(msg) = message {
                        self.handler_agent_message(msg).await;
                    }
                }
            }
        }
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        let title = Line::from("cooronx的超级简单coding agent")
            .bold()
            .blue()
            .centered();
        let trunks = Layout::vertical([Constraint::Percentage(90), Constraint::Percentage(10)])
            .split(frame.area());
        let mut lines = Vec::default();
        for message in &self.display_message {
            for line in message.content.split("\n") {
                lines.push(Line::from(line));
            }
            lines.push(Line::default());
        }
        let para1 = Paragraph::new(lines).block(Block::bordered().title(title));
        
        frame.render_widget(para1, trunks[0]);
        frame.render_widget(&self.inputs, trunks[1]);
        force_redraw_area(frame.buffer_mut(), trunks[1]);
    }

    async fn handle_crossterm_events(&mut self) -> Result<()> {
        let event = self.event_stream.next().fuse().await;

        match event {
            Some(Ok(evt)) => match evt {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    self.on_key_event(key).await?
                }
                _ => {}
            },
            _ => {}
        }

        Ok(())
    }

    async fn on_key_event(&mut self, key: KeyEvent) -> Result<()> {
        match (key.modifiers, key.code) {
            (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
                self.quit();
                Ok(())
            }
            (KeyModifiers::SHIFT, KeyCode::Enter) => {
                self.inputs.insert_newline();
                Ok(())
            }
            (_, KeyCode::Enter) => {
                // 后面用于给ai发送消息
                let msg = self.inputs.lines().join("\n");
                self.display_message.push(DisplayMessage { role: TuiUser, content: msg.clone() });
                self.message_sender
                    .send(Message::UserMessage(UserCommand::Submit(msg)))
                    .await?;
                // 发送完之后要清空输入框
                self.inputs.clear();
                Ok(())
            }
            _ => {
                self.inputs.input(key);
                Ok(())
            }
        }
    }

    async fn handler_agent_message(&mut self, msg: Message) {
        match msg {
            Message::AgentMessage(agent_event) => match agent_event {
                crate::types::AgentEvent::Started => {
                    // 新开一条消息，等于塞到最后面去
                    self.current_assitant_index = Some(self.display_message.len());
                    self.display_message.push(DisplayMessage {
                        role: Assistant,
                        content: String::default(),
                    });
                }
                crate::types::AgentEvent::Delta(s) => {
                    if let Some(index) = self.current_assitant_index {
                        if let Some(display_str) = self.display_message.get_mut(index) {
                            display_str.content.push_str(s.as_str());
                        }
                    }
                }
                crate::types::AgentEvent::Done => {
                    // 结束了就置空
                    self.current_assitant_index = None;
                }
                crate::types::AgentEvent::Error(s) => {
                    if let Some(index) = self.current_assitant_index {
                        if let Some(display_str) = self.display_message.get_mut(index) {
                            display_str.content.push_str(s.as_str());
                        }
                    }
                    self.current_assitant_index = None;
                }
            },
            Message::UserMessage(user_command) => {}
        }
    }

    fn quit(&mut self) {
        self.running = false;
    }
}

fn force_redraw_area(buffer: &mut Buffer, area: Rect) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.set_diff_option(CellDiffOption::AlwaysUpdate);
            }
        }
    }
}
