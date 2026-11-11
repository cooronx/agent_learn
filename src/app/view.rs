//! 整体布局渲染：主区域、右侧栏、会话列表。

use ratatui::{
    Frame,
    buffer::CellDiffOption,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Clear, List, ListItem, Paragraph, Wrap},
};

use super::App;
use super::text::{format_relative_time, truncate_left, truncate_right};

impl App<'_> {
    pub fn draw(&mut self, frame: &mut Frame) {
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

        // 输出速度：最近一次生成 + 本次会话平均
        let speed = match self.last_tokens_per_second {
            Some(value) => format!("{value:.1} token/s"),
            None => "—".to_string(),
        };
        let average_seconds = self.generation_elapsed.as_secs_f64();
        let average = if self.generated_tokens == 0 || average_seconds <= 0.0 {
            "平均 —".to_string()
        } else {
            format!(
                "平均 {:.1} token/s",
                self.generated_tokens as f64 / average_seconds
            )
        };

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
            Line::from(vec![
                Span::styled("当前  ", Style::default().fg(Color::DarkGray)),
                Span::styled(speed, Style::default().fg(Color::White).bold()),
            ]),
            Line::from(Span::styled(average, Style::default().fg(Color::DarkGray))),
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
}

/// 强制重绘，解决中文字符删除的时候会留下光标的问题(这真的是解决方法吗？)
fn force_redraw_area(buffer: &mut ratatui::buffer::Buffer, area: Rect) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.set_diff_option(CellDiffOption::AlwaysUpdate);
            }
        }
    }
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
