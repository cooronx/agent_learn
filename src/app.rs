use std::{path::PathBuf, time::Duration};

use color_eyre::Result;
use crossterm::event::{
    Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent,
};
use futures::StreamExt;
use ratatui::{
    DefaultTerminal, Frame,
    buffer::{Buffer, CellDiffOption},
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap},
};
use ratatui_textarea::TextArea;
use tokio::sync::mpsc;

use crate::{
    ai::types::{ModelMessage, ToolCall},
    session::{Session, SessionMeta},
    types::{
        AgentEvent,
        Role::{self, Assistant, User as TuiUser},
        ToolStatus, TurnResult, UserCommand,
    },
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
enum DisplayPart {
    Content(String),
    Reasoning(String),
    ToolCall(ToolCall),
    ToolResult(ToolResultView),
    Status(String),
}

#[derive(Debug)]
struct ToolResultView {
    status: ToolStatus,
    summary: String,
    preview: Vec<String>,
    truncated: bool,
    output: String,
}

#[derive(Debug)]
struct DisplayMessage {
    role: Role,
    parts: Vec<DisplayPart>,
}

impl DisplayMessage {
    fn new(role: Role) -> Self {
        Self {
            role,
            parts: Vec::new(),
        }
    }

    // 按照 delta 到达的顺序保存，保证渲染时 assistant msg 和 tool call 是交错的
    fn push_part(&mut self, part: DisplayPart) {
        match (self.parts.last_mut(), &part) {
            (Some(DisplayPart::Content(existing)), DisplayPart::Content(new))
            | (Some(DisplayPart::Reasoning(existing)), DisplayPart::Reasoning(new)) => {
                existing.push_str(new);
            }
            _ => self.parts.push(part),
        }
    }

    fn has_output_content(&self) -> bool {
        self.parts.iter().any(|part| match part {
            DisplayPart::Content(content) => !content.is_empty(),
            _ => false,
        })
    }
}

#[derive(Debug)]
pub struct App<'a> {
    running: bool,
    events: mpsc::Receiver<AgentEvent>,
    commands: mpsc::Sender<UserCommand>,
    display_message: Vec<DisplayMessage>,
    current_assistant_index: Option<usize>,
    inputs: TextArea<'a>,
    message_scroll: u16,
    message_max_scroll: u16,
    follow_tail: bool,
    reasoning_expanded: bool,
    tools_expanded: bool,
    spinner_frame: usize,
    busy: bool,
    stopping: bool,

    // ===== 右侧栏相关 =====
    /// 当前工作目录，启动时取一次即可
    working_dir: String,
    /// 会话标题，取用户发出的第一句话
    session_title: Option<String>,
    /// 会话文件所在目录
    session_dir: PathBuf,
    /// 会话选择器数据
    sessions: Vec<SessionMeta>,
    session_list_open: bool,
    session_list_state: ListState,
    session_list_error: Option<String>,
    /// 最近一次请求的输入 token 数（即当前上下文占用的 token 数）
    prompt_tokens: u64,
    /// 最近一次请求输入 + 输出累积的 token 数
    total_tokens: u64,
    /// 模型上下文窗口大小，用来计算百分比
    context_limit: u64,
}

impl App<'_> {
    pub fn new(
        sender: mpsc::Sender<UserCommand>,
        recv: mpsc::Receiver<AgentEvent>,
        context_limit: u64,
        session_dir: PathBuf,
    ) -> Self {
        let mut input = TextArea::default();
        input.set_block(
            Block::bordered().title("输入框").title_bottom(
                Line::from(Span::styled(
                    " Ctrl+S 会话 · Ctrl+O 工具 ",
                    Style::default().fg(Color::DarkGray),
                ))
                .alignment(Alignment::Right),
            ),
        );
        Self {
            running: false,
            events: recv,
            commands: sender,
            display_message: Vec::default(),
            current_assistant_index: None,
            inputs: input,
            message_scroll: 0,
            message_max_scroll: 0,
            follow_tail: false,
            // 2026.09.03 暂时一直设置为false，codex也没有展示给用户看思考过程，所以我们也没有必要
            reasoning_expanded: false,
            tools_expanded: false,
            spinner_frame: 0,
            busy: false,
            stopping: false,
            working_dir: std::env::current_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|_| "unknown".to_string()),
            session_title: None,
            session_dir,
            sessions: Vec::new(),
            session_list_open: false,
            session_list_state: ListState::default(),
            session_list_error: None,
            prompt_tokens: 0,
            total_tokens: 0,
            context_limit,
        }
    }

    pub async fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        self.running = true;
        let mut event_stream = EventStream::new();

        // thinking动画
        let mut spinner_tick = tokio::time::interval(Duration::from_millis(50));
        spinner_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        while self.running {
            terminal.draw(|frame| self.draw(frame))?;
            tokio::select! {

                // 动画
                _ = spinner_tick.tick() => {
                    if self.current_assistant_index.is_some() {
                        self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
                    }
                }


                event = event_stream.next() => {
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

                message = self.events.recv() => {
                        if let Some(msg) = message {
                        // 收到了新的消息，那么也要切换到消息最末尾
                        self.apply_event(msg);
                    } else {
                        self.running = false;
                    }
                }
            }
        }
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        // 先在水平方向切一刀：左侧主区域 + 右侧栏
        // 右侧栏固定 32 列，但如果终端太窄，则退化为 30% 宽度，保证主区域不被挤压
        let sidebar_width = if frame.area().width >= 80 {
            Constraint::Length(32)
        } else {
            Constraint::Percentage(30)
        };
        let columns = Layout::horizontal([Constraint::Min(0), sidebar_width]).split(frame.area());

        let main_area = columns[0];
        let sidebar_area = columns[1];

        // 主区域再垂直分割：消息列表 + 输入框
        let trunks = Layout::vertical([Constraint::Min(0), Constraint::Length(3)]).split(main_area);

        // 生成需要渲染的line（以及填充格式）
        let lines = self.gen_main_lines();

        let para_main = Paragraph::new(lines).wrap(Wrap { trim: false });

        let title = Line::from("cooronx的超级简单coding agent")
            .bold()
            .blue()
            .centered();

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

        // 渲染右侧栏（Context / token / 工作目录 / 标题）
        self.draw_sidebar(frame, sidebar_area);

        if self.session_list_open {
            self.draw_session_list(frame);
        }

        self.message_scroll = current_scoll;
        self.message_max_scroll = max_scorll;
    }

    fn draw_session_list(&mut self, frame: &mut Frame) {
        let area = centered_rect(70, 60, frame.area());
        if area.width < 20 || area.height < 5 {
            return;
        }
        frame.render_widget(Clear, area);

        let block = Block::bordered()
            .title(Line::from(format!(" 会话 ({}) ", self.sessions.len())).bold())
            .border_style(Style::default().fg(Color::Cyan));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let chunks = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);
        let list_area = chunks[0];
        let footer_area = chunks[1];

        if self.sessions.is_empty() {
            let message = self
                .session_list_error
                .as_deref()
                .unwrap_or("暂无会话，按 n 新建");
            let color = if self.session_list_error.is_some() {
                Color::Red
            } else {
                Color::DarkGray
            };
            frame.render_widget(
                Paragraph::new(message)
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(color)),
                list_area,
            );
        } else {
            let width = list_area.width as usize;
            let items: Vec<ListItem> = self
                .sessions
                .iter()
                .map(|meta| {
                    let time = format_relative_time(meta.created_at);
                    let title = if meta.title.trim().is_empty() {
                        "(无标题)".to_string()
                    } else {
                        meta.title.replace('\n', " ")
                    };
                    let content_width = width.saturating_sub(2);
                    let max_title = content_width.saturating_sub(time.chars().count() + 2);
                    let title = truncate_right(&title, max_title);
                    let padding =
                        content_width.saturating_sub(title.chars().count() + time.chars().count());

                    ListItem::new(Line::from(vec![
                        Span::styled(title, Style::default().fg(Color::White)),
                        Span::raw(" ".repeat(padding)),
                        Span::styled(time, Style::default().fg(Color::DarkGray)),
                    ]))
                })
                .collect();

            let list = List::new(items).highlight_symbol("› ").highlight_style(
                Style::default()
                    .bg(Color::Rgb(30, 45, 55))
                    .fg(Color::Cyan)
                    .bold(),
            );

            frame.render_stateful_widget(list, list_area, &mut self.session_list_state);
        }

        let footer = if let Some(error) = &self.session_list_error {
            Line::from(Span::styled(
                format!(" {error} "),
                Style::default().fg(Color::Red),
            ))
        } else {
            Line::from(Span::styled(
                " ↑/↓ 选择 · Enter 恢复 · n 新建 · Esc 关闭 ",
                Style::default().fg(Color::DarkGray),
            ))
        };
        frame.render_widget(Paragraph::new(footer), footer_area);
    }

    /// 渲染右侧边栏
    fn draw_sidebar(&self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered().title(Line::from("Context").bold().cyan());
        let inner = block.inner(area);
        frame.render_widget(block, area);

        // 计算上下文占用百分比（输入 token / 上下文窗口）
        let percent = if self.context_limit == 0 {
            0.0
        } else {
            self.prompt_tokens as f64 / self.context_limit as f64 * 100.0
        };

        // 简单的 token 条形图：宽度按 inner 宽度（减去一些留白）缩放
        let bar_width = inner.width.saturating_sub(2) as usize;
        let filled = if self.context_limit == 0 {
            0
        } else {
            ((percent / 100.0) * bar_width as f64).round() as usize
        }
        .min(bar_width);
        let bar_color = if percent >= 90.0 {
            Color::Red
        } else if percent >= 70.0 {
            Color::Yellow
        } else {
            Color::Green
        };
        let bar = Line::from(vec![
            Span::styled("█".repeat(filled), Style::default().fg(bar_color)),
            Span::styled(
                "░".repeat(bar_width - filled),
                Style::default().fg(Color::DarkGray),
            ),
        ]);

        // 工作目录：太长时从左侧截断，优先保留尾部（更有信息量）
        let dir = truncate_left(&self.working_dir, inner.width.saturating_sub(1) as usize);

        // 标题：取用户第一句话，去掉换行并截断
        let title = self
            .session_title
            .clone()
            .unwrap_or_else(|| "(新对话)".to_string());
        let title = truncate_right(&title, inner.width.saturating_sub(1) as usize);

        let lines = vec![
            Line::from(vec![
                Span::styled("Token  ", Style::default().fg(Color::DarkGray)),
                Span::styled(
                    format!("{} / {}", self.prompt_tokens, self.context_limit),
                    Style::default().fg(Color::White).bold(),
                ),
            ]),
            Line::from(vec![
                Span::styled(
                    format!("{:>5.1}%", percent),
                    Style::default().fg(bar_color).bold(),
                ),
                Span::raw("  "),
                Span::styled(
                    format!("(本次总计 {})", self.total_tokens),
                    Style::default().fg(Color::DarkGray),
                ),
            ]),
            bar,
            Line::default(),
            Line::from(Span::styled(
                "工作目录",
                Style::default().fg(Color::DarkGray),
            )),
            Line::from(Span::styled(dir, Style::default().fg(Color::White))),
            Line::default(),
            Line::from(Span::styled("标题", Style::default().fg(Color::DarkGray))),
            Line::from(Span::styled(title, Style::default().fg(Color::White))),
        ];

        let para = Paragraph::new(lines).wrap(Wrap { trim: false });
        frame.render_widget(para, inner);
    }

    async fn on_key_event(&mut self, key: KeyEvent) -> Result<()> {
        if self.session_list_open {
            return self.on_session_list_key(key).await;
        }
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
                // 先把思考展示这个功能关闭了
                // self.reasoning_expanded = !self.reasoning_expanded;
                Ok(())
            }
            (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
                self.tools_expanded = !self.tools_expanded;
                Ok(())
            }
            (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
                if !self.busy {
                    self.refresh_sessions();
                    self.session_list_open = true;
                }
                Ok(())
            }
            (KeyModifiers::CONTROL, KeyCode::Char('n')) => {
                if !self.busy {
                    self.commands.send(UserCommand::NewSession).await?;
                    self.busy = true;
                }
                Ok(())
            }
            (_, KeyCode::Esc) => {
                // 用户主动终止了对话
                if self.busy && !self.stopping {
                    self.commands.send(UserCommand::Cancel).await?;
                    self.stopping = true;
                }
                Ok(())
            }
            (_, KeyCode::Enter) => {
                // 后面用于给ai发送消息
                if self.busy {
                    return Ok(());
                }
                let msg = self.inputs.lines().join("\n");
                if msg.trim().is_empty() {
                    return Ok(());
                }
                if let Err(err) = self.commands.send(UserCommand::Submit(msg.clone())).await {
                    self.display_message.push(DisplayMessage {
                        role: Role::Error,
                        parts: vec![DisplayPart::Status(err.to_string())],
                    });
                    return Ok(());
                }
                self.busy = true;
                // 如果是第一条用户消息，则把它作为本次对话的标题
                if self.session_title.is_none() && !msg.trim().is_empty() {
                    // 只取第一行，避免多行输入把标题撑爆
                    let first_line = msg.lines().next().unwrap_or("").trim();
                    self.session_title = Some(first_line.to_string());
                }
                self.display_message.push(DisplayMessage {
                    parts: vec![DisplayPart::Content(msg.clone())],
                    role: TuiUser,
                });
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

    async fn on_session_list_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
            self.quit();
            return Ok(());
        }

        match key.code {
            KeyCode::Esc => self.session_list_open = false,
            KeyCode::Up => self.select_session(-1),
            KeyCode::Down => self.select_session(1),
            KeyCode::Char('n') | KeyCode::Char('N') => {
                self.commands.send(UserCommand::NewSession).await?;
                self.session_list_open = false;
                self.busy = true;
            }
            KeyCode::Enter => {
                let path = self.session_list_state.selected().and_then(|index| {
                    self.sessions
                        .get(index)
                        .map(|meta| self.session_dir.join(format!("{}.jsonl", meta.id)))
                });
                if let Some(path) = path {
                    self.commands.send(UserCommand::Resume(path)).await?;
                    self.session_list_open = false;
                    self.busy = true;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn refresh_sessions(&mut self) {
        match Session::list(&self.session_dir) {
            Ok(sessions) => {
                self.sessions = sessions;
                self.session_list_error = None;
                if self.sessions.is_empty() {
                    self.session_list_state.select(None);
                } else {
                    self.session_list_state.select(Some(0));
                }
            }
            Err(error) => {
                self.sessions.clear();
                self.session_list_state.select(None);
                self.session_list_error = Some(error.to_string());
            }
        }
    }

    fn select_session(&mut self, delta: isize) {
        let len = self.sessions.len();
        if len == 0 {
            return;
        }
        let current = self.session_list_state.selected().unwrap_or(0);
        let next = if delta < 0 {
            if current == 0 { len - 1 } else { current - 1 }
        } else if current + 1 >= len {
            0
        } else {
            current + 1
        };
        self.session_list_state.select(Some(next));
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

    fn apply_event(&mut self, event: AgentEvent) {
        let part = match event {
            AgentEvent::TurnStarted => {
                self.busy = true;
                self.current_assistant_index = Some(self.display_message.len());
                self.display_message.push(DisplayMessage::new(Assistant));
                None
            }
            AgentEvent::SessionCleared => {
                self.session_list_open = false;
                self.display_message.clear();
                self.current_assistant_index = None;
                self.session_title = None;
                self.prompt_tokens = 0;
                self.total_tokens = 0;
                self.busy = false;
                self.stopping = false;
                self.follow_tail = true;
                None
            }
            AgentEvent::SessionLoaded { meta, messages } => {
                self.session_list_open = false;
                self.display_message = display_history(messages);
                self.current_assistant_index = None;
                self.session_title = if meta.title.trim().is_empty() {
                    None
                } else {
                    Some(meta.title)
                };
                self.prompt_tokens = 0;
                self.total_tokens = 0;
                self.busy = false;
                self.stopping = false;
                self.follow_tail = true;
                None
            }
            AgentEvent::SessionLoadFailed(error) => {
                self.session_list_open = false;
                self.display_message.push(DisplayMessage {
                    role: Role::Error,
                    parts: vec![DisplayPart::Status(format!("会话加载失败：{error}"))],
                });
                self.busy = false;
                self.stopping = false;
                self.follow_tail = true;
                None
            }
            AgentEvent::TitleUpdated(title) => {
                self.session_title = Some(title);
                None
            }
            AgentEvent::TextDelta(text) => Some(DisplayPart::Content(text)),
            AgentEvent::ReasoningDelta(text) => Some(DisplayPart::Reasoning(text)),
            AgentEvent::ToolStarted(call) => Some(DisplayPart::ToolCall(call)),
            AgentEvent::ToolResult {
                call_id: _call_id,
                output,
                status,
            } => Some(DisplayPart::ToolResult(ToolResultView::new(status, output))),
            AgentEvent::UsageUpdated(usage) => {
                self.prompt_tokens = usage.prompt_tokens;
                self.total_tokens = usage.total_tokens;
                None
            }
            AgentEvent::TurnFinished(outcome) => {
                let status = match outcome {
                    TurnResult::Completed => None,
                    TurnResult::Cancelled => Some("已中断".to_string()),
                    TurnResult::Failed(error) => Some(format!("失败：{error}")),
                };
                if let Some(status) = status {
                    self.push_current(DisplayPart::Status(status));
                }
                self.current_assistant_index = None;
                self.busy = false;
                self.stopping = false;
                None
            }
        };
        if let Some(part) = part {
            self.push_current(part);
        }
    }

    fn push_current(&mut self, part: DisplayPart) {
        if let Some(index) = self.current_assistant_index {
            if let Some(message) = self.display_message.get_mut(index) {
                message.push_part(part);
            }
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

            if self.stopping && self.current_assistant_index == Some(index) {
                lines.push(Line::from("正在停止，等待当前工具完成…").yellow());
            }
            let is_current = self.current_assistant_index == Some(index);
            let is_thinking_animation =
                matches!(message.role, Assistant) && is_current && !message.has_output_content();

            if is_thinking_animation {
                let frame = SPINNER_FRAMES[self.spinner_frame];
                lines.push(Line::from(vec![Span::styled(
                    format!("{frame} Thinking..."),
                    Style::default().fg(Color::DarkGray).italic(),
                )]))
            }

            // 按照 delta 到达的顺序渲染：assistant msg -> tool call -> assistant msg -> ...
            for part in &message.parts {
                match part {
                    // reasoning 和最终回答之间留一行
                    DisplayPart::Reasoning(reasoning) => {
                        if !self.reasoning_expanded {
                            continue;
                        }
                        if let Some(reasoning_style) = reasoning_style {
                            for reasoning_line in reasoning.split('\n') {
                                lines.push(Line::from(vec![
                                    Span::styled("┊ ", reasoning_style),
                                    Span::styled(reasoning_line, reasoning_style),
                                ]));
                            }
                            lines.push(Line::default());
                        }
                    }
                    DisplayPart::Status(status) => {
                        lines.push(Line::from(status.as_str()).yellow());
                    }
                    DisplayPart::ToolCall(call) => {
                        let tool_style = Style::default().fg(Color::Yellow).italic();
                        lines.push(Line::from(vec![
                            Span::styled("⚙ ", tool_style),
                            Span::styled(summarize_tool_call(call), tool_style),
                        ]));
                    }
                    DisplayPart::ToolResult(view) => {
                        let (icon, color) = match view.status {
                            ToolStatus::Completed => ("✓", Color::Green),
                            ToolStatus::Failed => ("✗", Color::Red),
                            ToolStatus::Cancelled => ("⊘", Color::Yellow),
                        };
                        let hint = if view.truncated {
                            if self.tools_expanded {
                                " · Ctrl+O 收起"
                            } else {
                                " · Ctrl+O 展开"
                            }
                        } else {
                            ""
                        };

                        lines.push(Line::from(vec![
                            Span::raw("  "),
                            Span::styled(format!("{icon} "), Style::default().fg(color).bold()),
                            Span::styled(
                                format!("{}{hint}", view.summary),
                                Style::default().fg(Color::DarkGray),
                            ),
                        ]));

                        let output_style = match view.status {
                            ToolStatus::Failed => Style::default().fg(Color::LightRed),
                            _ => Style::default().fg(Color::DarkGray),
                        };

                        if self.tools_expanded {
                            for line in view.output.lines() {
                                lines.push(Line::from(vec![
                                    Span::styled("  │ ", output_style),
                                    Span::styled(line, output_style),
                                ]));
                            }
                        } else {
                            for line in &view.preview {
                                lines.push(Line::from(vec![
                                    Span::styled("  │ ", output_style),
                                    Span::styled(line.clone(), output_style),
                                ]));
                            }
                        }
                        lines.push(Line::default());
                    }
                    DisplayPart::Content(content) => match &message.role {
                        Assistant => {
                            // ai的最终回答用markdown来渲染一下，codex也是这样的
                            let markdown_style = tui_markdown::Options::new(CustomMarkdownStyle);

                            let markdown =
                                tui_markdown::from_str_with_options(content, &markdown_style);
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
                            for (line_index, content) in content.split('\n').enumerate() {
                                let current_prefix = if line_index == 0 { prefix } else { "  " };

                                lines.push(Line::from(vec![
                                    Span::styled(current_prefix, prefix_style),
                                    Span::styled(content, content_style),
                                ]));
                            }
                        }
                    },
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

/// 从左侧截断字符串，超出长度时保留尾部并加省略号（适用于路径）
fn truncate_left(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    // 保留 max-1 个尾部字符，前面加省略号
    let keep = max.saturating_sub(1);
    let tail: String = chars[chars.len() - keep..].iter().collect();
    format!("…{tail}")
}

/// 从右侧截断字符串，超出长度时保留头部并加省略号（适用于标题）
fn truncate_right(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let head: String = chars[..keep].iter().collect();
    format!("{head}…")
}

const TOOL_PREVIEW_WIDTH: usize = 100;

impl ToolResultView {
    fn new(status: ToolStatus, raw_output: String) -> Self {
        let (exit_summary, output) = decode_tool_output(raw_output);
        let lines: Vec<&str> = output.lines().collect();
        let total = lines.len();

        let (preview_limit, from_end) = match status {
            ToolStatus::Completed => (2, false),
            ToolStatus::Failed => (3, true),
            ToolStatus::Cancelled => (0, false),
        };

        let horizontally_truncated = lines
            .iter()
            .any(|line| line.chars().count() > TOOL_PREVIEW_WIDTH);
        let truncated =
            status != ToolStatus::Cancelled && (total > preview_limit || horizontally_truncated);

        let preview = if status == ToolStatus::Cancelled {
            Vec::new()
        } else if from_end && total > preview_limit {
            lines[total - preview_limit..]
                .iter()
                .map(|line| truncate_right(line, TOOL_PREVIEW_WIDTH))
                .collect()
        } else {
            lines
                .iter()
                .take(preview_limit)
                .map(|line| truncate_right(line, TOOL_PREVIEW_WIDTH))
                .collect()
        };

        let summary = if status == ToolStatus::Cancelled {
            "未执行".to_string()
        } else {
            let mut parts = Vec::new();
            if let Some(exit) = exit_summary {
                parts.push(exit);
            }
            match total {
                0 => parts.push("no output".to_string()),
                1 => parts.push("1 line".to_string()),
                n => parts.push(format!("{n} lines")),
            }
            parts.join(" · ")
        };

        Self {
            status,
            summary,
            preview,
            truncated,
            output,
        }
    }
}

/// bash_command 的返回值是 JSON，折叠展示时只取 stdout/stderr 和退出码。
fn decode_tool_output(raw: String) -> (Option<String>, String) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return (None, raw);
    };
    let Some(object) = value.as_object() else {
        return (None, raw);
    };
    let Some(stdout) = object.get("stdout").and_then(|value| value.as_str()) else {
        return (None, raw);
    };
    let stderr = object
        .get("stderr")
        .and_then(|value| value.as_str())
        .unwrap_or_default();

    let mut output = String::new();
    output.push_str(stdout);
    if !stderr.is_empty() {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str(stderr);
    }

    let tool_truncated = object
        .get("is_stdout_truncated")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
        || object
            .get("is_stderr_truncated")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
    if tool_truncated {
        if !output.is_empty() && !output.ends_with('\n') {
            output.push('\n');
        }
        output.push_str("[output truncated by tool]");
    }

    let exit = Some(
        match object.get("exit_code").and_then(|value| value.as_i64()) {
            Some(code) => format!("exit {code}"),
            None => "exit ?".to_string(),
        },
    );
    (exit, output)
}

fn summarize_tool_call(call: &ToolCall) -> String {
    let args = &call.arguments;
    let summary = match call.name.as_str() {
        "read_file" => {
            let path = args
                .get("path")
                .and_then(|value| value.as_str())
                .unwrap_or("?");
            let offset = args.get("offset").and_then(|value| value.as_u64());
            let limit = args.get("limit").and_then(|value| value.as_u64());
            match (offset, limit) {
                (Some(offset), Some(limit)) => {
                    format!("{path}:{}-{}", offset + 1, offset + limit)
                }
                (Some(offset), None) => format!("{path}:{}", offset + 1),
                (None, Some(limit)) => format!("{path}:1-{limit}"),
                (None, None) => path.to_string(),
            }
        }
        "bash_command" => {
            let command = args
                .get("command")
                .and_then(|value| value.as_str())
                .unwrap_or("?");
            format!("$ {}", command.replace('\n', " ; "))
        }
        "edit" => {
            let path = args
                .get("path")
                .and_then(|value| value.as_str())
                .unwrap_or("?");
            let edits = args
                .get("edits")
                .and_then(|value| value.as_array())
                .map(|edits| edits.len())
                .unwrap_or(0);
            format!("{path} ({edits} edits)")
        }
        "write" => args
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or("?")
            .to_string(),
        "ls" => args
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or(".")
            .to_string(),
        _ => serde_json::to_string(args).unwrap_or_default(),
    };

    let summary = truncate_right(&summary, TOOL_PREVIEW_WIDTH);
    if summary.is_empty() {
        call.name.clone()
    } else {
        format!("{} {}", call.name, summary)
    }
}

fn display_history(messages: Vec<ModelMessage>) -> Vec<DisplayMessage> {
    let mut display = Vec::new();

    for message in messages {
        match message {
            ModelMessage::System(_) => {}
            ModelMessage::User(user) => {
                display.push(DisplayMessage {
                    role: TuiUser,
                    parts: vec![DisplayPart::Content(user.content)],
                });
            }
            ModelMessage::Assistant(assistant) => {
                if !display
                    .last()
                    .is_some_and(|message| matches!(&message.role, Assistant))
                {
                    display.push(DisplayMessage::new(Assistant));
                }
                let current = display
                    .last_mut()
                    .expect("assistant display was just pushed");

                if let Some(content) = assistant.content {
                    if !content.is_empty() {
                        current.push_part(DisplayPart::Content(content));
                    }
                }
                if let Some(reasoning) = assistant.reasoning {
                    if !reasoning.is_empty() {
                        current.push_part(DisplayPart::Reasoning(reasoning));
                    }
                }
                if let Some(tool_calls) = assistant.tool_calls {
                    for call in tool_calls {
                        current.push_part(DisplayPart::ToolCall(call));
                    }
                }
            }
            ModelMessage::ToolResult(result) => {
                if !display
                    .last()
                    .is_some_and(|message| matches!(&message.role, Assistant))
                {
                    display.push(DisplayMessage::new(Assistant));
                }
                let status = if result.content.starts_with("tool execution failed:") {
                    ToolStatus::Failed
                } else if result.content.starts_with("未执行") {
                    ToolStatus::Cancelled
                } else {
                    ToolStatus::Completed
                };
                let current = display
                    .last_mut()
                    .expect("assistant display was just pushed");
                current.push_part(DisplayPart::ToolResult(ToolResultView::new(
                    status,
                    result.content,
                )));
            }
        }
    }

    display
}

fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    let horizontal = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1]);
    horizontal[1]
}

fn format_relative_time(created_at: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let seconds = now.saturating_sub(created_at) / 1000;

    match seconds {
        0..=59 => "刚刚".to_string(),
        60..=3599 => format!("{} 分钟前", seconds / 60),
        3600..=86_399 => format!("{} 小时前", seconds / 3600),
        _ => format!("{} 天前", seconds / 86_400),
    }
}
