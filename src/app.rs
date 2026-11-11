//! TUI 应用外壳。
//!
//! 这里只保留「状态 + 事件循环 + 事件状态迁移」，
//! 具体渲染、输入处理、消息展示分别放到子模块里：
//!
//! - `input`       键盘 / 鼠标事件
//! - `view`        整体布局、右侧栏、会话列表
//! - `message_view` 对话消息 -> Line 的渲染
//! - `display`     展示层数据模型
//! - `text`        字符串截断等小工具

mod display;
mod input;
mod message_view;
mod text;
mod view;

use std::{path::PathBuf, time::Duration};

use crossterm::event::{Event, EventStream, KeyEventKind};
use futures::StreamExt;
use ratatui::{
    DefaultTerminal,
    layout::Alignment,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, ListState},
};
use ratatui_textarea::TextArea;
use tokio::sync::mpsc;

use crate::{
    session::SessionMeta,
    types::{AgentEvent, Role::Assistant, TurnResult, UserCommand},
};

use display::{DisplayMessage, DisplayPart, ToolResultView, display_history};

const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

#[derive(Debug)]
pub struct App<'a> {
    running: bool,
    needs_redraw: bool,
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
            needs_redraw: true,
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
            if self.needs_redraw {
                terminal.draw(|frame| self.draw(frame))?;
                self.needs_redraw = false;
            }

            tokio::select! {
                // 动画只在有任务时运行；空闲时不重绘，避免输入延迟
                _ = spinner_tick.tick(), if self.busy || self.stopping || self.current_assistant_index.is_some() => {
                    self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
                    self.needs_redraw = true;
                }

                event = event_stream.next() => {
                    match event {
                        Some(Ok(Event::Key(key))) => {
                            if key.kind == KeyEventKind::Press {
                                self.on_key_event(key).await?;
                                self.needs_redraw = true;
                            }
                        },
                        Some(Ok(Event::Mouse(mouse))) => {
                            self.on_mouse_event(mouse).await?;
                            self.needs_redraw = true;
                        }
                        Some(Ok(Event::Resize(_, _))) => {
                            self.needs_redraw = true;
                        }
                        _ => {},
                    }
                }

                message = self.events.recv() => {
                    if let Some(msg) = message {
                        // 尽量一次处理完积压的事件，避免每个 token 都重绘一次
                        self.apply_event(msg);
                        while let Ok(msg) = self.events.try_recv() {
                            self.apply_event(msg);
                        }
                        self.needs_redraw = true;
                    } else {
                        self.running = false;
                    }
                }
            }
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
                    role: crate::types::Role::Error,
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
}
