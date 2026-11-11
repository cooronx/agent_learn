//! 渲染文本

use ratatui::{
    style::{Color, Style, Stylize},
    text::{Line, Span},
};

use crate::types::{
    Role::{self, Assistant, User as TuiUser},
    ToolStatus,
};

use super::App;
use super::SPINNER_FRAMES;
use super::display::{DisplayPart, summarize_tool_call};

// markdown的样式配置
#[derive(Clone)]
struct CustomMarkdownStyle;

impl tui_markdown::StyleSheet for CustomMarkdownStyle {
    fn heading_marker(&self, _: u8) -> &str {
        ""
    }
}

impl App<'_> {
    pub fn gen_main_lines(&self) -> Vec<Line<'_>> {
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
