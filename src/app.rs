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
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
};
use ratatui_textarea::{TextArea, WrapMode};
use tokio::sync::mpsc;

use crate::types::{
    Message,
    Role::{self, Assistant, User as TuiUser},
    UserCommand,
};

#[derive(Debug)]
struct DisplayMessage {
    role: Role,
    reasoning_content: String,
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
            let (prefix, prefix_style, content_style, reasoning_style) = match message.role {
                TuiUser => (
                    "› ",
                    Style::default().fg(Color::Cyan).bg(Color::Green).bold(),
                    Style::default().fg(Color::White).bg(Color::Green),
                    None,
                ),
                Assistant => (
                    "• ",
                    Style::default().fg(Color::Green).bold(),
                    Style::default().fg(Color::White),
                    Some(Style::default().fg(Color::DarkGray).italic()),
                ),
                Role::Error => (
                    "× ",
                    Style::default().fg(Color::Red).bold(),
                    Style::default().fg(Color::LightRed),
                    None,
                ),
            };
            // 渲染当前这条消息的 reasoning
            if !message.reasoning_content.is_empty() {
                if let Some(reasoning_style) = reasoning_style {
                    for reasoning_line in message.reasoning_content.split('\n') {
                        lines.push(Line::from(vec![
                            Span::styled("┊ ", reasoning_style),
                            Span::styled(reasoning_line, reasoning_style),
                        ]));
                    }

                    // reasoning 和最终回答之间留一行
                    if !message.content.is_empty() {
                        lines.push(Line::default());
                    }
                }
            }
            for (line_index, content) in message.content.split("\n").enumerate() {
                let current_prefix = if line_index == 0 { prefix } else { "  " };
                lines.push(Line::from(vec![
                    Span::styled(current_prefix, prefix_style),
                    Span::styled(content, content_style),
                ]));
            }
            lines.push(Line::default());
        }
        let para_main = Paragraph::new(lines)
            .block(Block::bordered().title(title))
            .wrap(Wrap { trim: false });

        frame.render_widget(para_main, trunks[0]);
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
                self.display_message.push(DisplayMessage {
                    role: TuiUser,
                    content: msg.clone(),
                    reasoning_content: String::default(),
                });
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
                        reasoning_content: String::default(),
                    });
                }
                crate::types::AgentEvent::Delta(s) => {
                    if let Some(index) = self.current_assitant_index {
                        if let Some(display_str) = self.display_message.get_mut(index) {
                            match s {
                                crate::types::ChoiceDelta::OutputDelta(output_str) => {
                                    display_str.content.push_str(&output_str);
                                }
                                crate::types::ChoiceDelta::ReasoningDelta(reasoning_str) => {
                                    display_str.reasoning_content.push_str(&reasoning_str);
                                }
                            };
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
