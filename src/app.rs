use std::time::Duration;

use color_eyre::Result;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent},
    execute,
    terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate},
};
use futures::{FutureExt, StreamExt};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::{Buffer, CellDiffOption},
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    symbols::line,
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

const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

// markdown的样式配置
#[derive(Clone)]
struct CustomMarkdownStyle;

impl tui_markdown::StyleSheet for CustomMarkdownStyle {
    fn heading_marker(&self, _: u8) -> &str {
        ""
    }
}

#[derive(Debug)]
struct DisplayMessage {
    role: Role,
    reasoning_content: String,
    content: String,
    tool_call_content: String,
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
    message_scroll: u16,
    message_max_scroll: u16,
    follow_tail: bool,
    reasoning_expanded: bool,
    spinner_frame: usize,
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
            message_scroll: 0,
            message_max_scroll: 0,
            follow_tail: false,
            reasoning_expanded: false,
            spinner_frame: 0,
        }
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        self.running = true;

        // thinking动画
        let mut spinner_tick = tokio::time::interval(Duration::from_millis(50));
        spinner_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        while self.running {
            terminal.draw(|frame| self.draw(frame))?;
            tokio::select! {

                // 动画
                _ = spinner_tick.tick() => {
                    if self.current_assitant_index.is_some() {
                        self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
                    }
                }


                event = self.event_stream.next() => {
                match event {
                    Some(Ok(Event::Key(key))) => {
                        if key.kind == KeyEventKind::Press {
                            self.on_key_event(key).await?
                        }
                    },
                    Some(Ok(Event::Mouse(mouse))) => {
                        self.on_mouse_event(mouse).await?
                    }
                    _ => {},
                }
              }

                message = self.message_recv.recv() => {
                        if let Some(msg) = message {
                        // 收到了新的消息，那么也要切换到消息最末尾
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

        // 生成需要渲染的line（以及填充格式）
        let lines = self.gen_main_lines();

        let para_main = Paragraph::new(lines).wrap(Wrap { trim: false });

        let area = trunks[0];
        let content_width = area.width.saturating_sub(2);
        let content_height = para_main.line_count(content_width);
        let viewport_height = area.height.saturating_sub(2) as usize;

        let max_scorll = content_height
            .saturating_sub(viewport_height)
            .min(u16::MAX as usize) as u16;
        let current_scoll = self.message_scroll.min(max_scorll);
        let current_scoll = if self.follow_tail {
            max_scorll
        } else {
            current_scoll
        };

        let para_main = para_main
            .block(Block::bordered().title(title))
            .scroll((current_scoll, 0));

        frame.render_widget(para_main, trunks[0]);
        frame.render_widget(&self.inputs, trunks[1]);
        force_redraw_area(frame.buffer_mut(), trunks[1]);
        self.message_scroll = current_scoll;
        self.message_max_scroll = max_scorll;
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
            (KeyModifiers::CONTROL, KeyCode::Char('t')) => {
                self.reasoning_expanded = !self.reasoning_expanded;
                Ok(())
            }
            (_, KeyCode::Enter) => {
                // 后面用于给ai发送消息
                let msg = self.inputs.lines().join("\n");
                self.display_message.push(DisplayMessage {
                    role: TuiUser,
                    content: msg.clone(),
                    reasoning_content: String::default(),
                    tool_call_content: String::default(),
                });
                self.message_sender
                    .send(Message::UserMessage(UserCommand::Submit(msg)))
                    .await?;
                // 发送消息后将输出框定位到最后一条消息
                self.follow_tail = true;
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

    async fn on_mouse_event(&mut self, mouse_event: MouseEvent) -> Result<()> {
        // 一次滑动多少行
        let mouse_scorll_lines = 2;

        match mouse_event.kind {
            crossterm::event::MouseEventKind::ScrollDown => {
                self.message_scroll = self
                    .message_scroll
                    .saturating_add(mouse_scorll_lines)
                    .min(self.message_max_scroll);
                self.follow_tail = self.message_scroll == self.message_max_scroll;
            }
            crossterm::event::MouseEventKind::ScrollUp => {
                self.message_scroll = self.message_scroll.saturating_sub(mouse_scorll_lines);
                self.follow_tail = false;
            }
            _ => {}
        }
        Ok(())
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
                        tool_call_content: String::default(),
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
                                crate::types::ChoiceDelta::ToolCallContent(tool_call_str) => {
                                    if !display_str.tool_call_content.is_empty() {
                                        display_str.tool_call_content.push_str("\n");
                                    }
                                    display_str.tool_call_content.push_str(&tool_call_str);
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

    fn gen_main_lines(&self) -> Vec<Line<'_>> {
        let mut lines = Vec::default();
        // 每次都遍历所有可展示信息（对话长了之后，对性能影响很大）
        for (index, message) in self.display_message.iter().enumerate() {
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

            let is_current = self.current_assitant_index == Some(index);
            let is_thinking_animation =
                matches!(message.role, Assistant) && is_current && message.content.is_empty();

            if is_thinking_animation {
                let frame = SPINNER_FRAMES[self.spinner_frame];
                lines.push(Line::from(vec![Span::styled(
                    format!("{frame} Thinking..."),
                    Style::default().fg(Color::DarkGray).italic(),
                )]))
            }

            // 选渲染当前这条消息的 reasoning
            if self.reasoning_expanded && !message.reasoning_content.is_empty() {
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

            // 渲染工具调用
            if !message.tool_call_content.is_empty() {
                let tool_style = Style::default().fg(Color::Yellow).italic();

                for call in message.tool_call_content.split('\n') {
                    lines.push(Line::from(vec![
                        Span::styled("👀 ", tool_style),
                        Span::styled(call, tool_style),
                    ]));
                }
                lines.push(Line::default());
            }

            // 再渲染output
            if !message.content.is_empty() {
                match &message.role {
                    Assistant => {
                        // ai的最终回答用markdown来渲染一下，codex也是这样的
                        let markdown_style = tui_markdown::Options::new(CustomMarkdownStyle);

                        let markdown =
                            tui_markdown::from_str_with_options(&message.content, &markdown_style);
                        for (line_index, mut markdown_line) in
                            markdown.lines.into_iter().enumerate()
                        {
                            let current_prefix = if line_index == 0 { "• " } else { "  " };
                            markdown_line
                                .spans
                                .insert(0, Span::styled(current_prefix, prefix_style));

                            lines.push(markdown_line);
                        }
                    }
                    _ => {
                        for (line_index, content) in message.content.split('\n').enumerate() {
                            let current_prefix = if line_index == 0 { prefix } else { "  " };

                            lines.push(Line::from(vec![
                                Span::styled(current_prefix, prefix_style),
                                Span::styled(content, content_style),
                            ]));
                        }
                    }
                }
            }
            lines.push(Line::default());
        }
        lines
    }
}

/// 强制重绘，解决中文字符删除的时候会留下光标的问题(这真的是解决方法吗？)
fn force_redraw_area(buffer: &mut Buffer, area: Rect) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.set_diff_option(CellDiffOption::AlwaysUpdate);
            }
        }
    }
}
