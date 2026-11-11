//! 键盘 / 鼠标输入处理。

use color_eyre::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent};

use crate::{
    session::Session,
    types::{
        Role::{self, User as TuiUser},
        UserCommand,
    },
};

use super::App;
use super::display::{DisplayMessage, DisplayPart};

impl App<'_> {
    pub async fn on_key_event(&mut self, key: KeyEvent) -> Result<()> {
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

    pub async fn on_mouse_event(&mut self, mouse_event: MouseEvent) -> Result<()> {
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
}
