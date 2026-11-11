//! 展示层数据模型：把 Agent 事件 / 历史消息整理成可直接渲染的结构。

use crate::{
    ai::types::{ModelMessage, ToolCall},
    types::{
        Role::{self, Assistant, User as TuiUser},
        ToolStatus,
    },
};

use super::text::{TOOL_PREVIEW_WIDTH, truncate_right};

#[derive(Debug)]
pub enum DisplayPart {
    Content(String),
    Reasoning(String),
    ToolCall(ToolCall),
    ToolResult(ToolResultView),
    Status(String),
}

#[derive(Debug)]
pub struct ToolResultView {
    pub status: ToolStatus,
    pub summary: String,
    pub preview: Vec<String>,
    pub truncated: bool,
    pub output: String,
}

#[derive(Debug)]
pub struct DisplayMessage {
    pub role: Role,
    pub parts: Vec<DisplayPart>,
}

impl DisplayMessage {
    pub fn new(role: Role) -> Self {
        Self {
            role,
            parts: Vec::new(),
        }
    }

    // 按照 delta 到达的顺序保存，保证渲染时 assistant msg 和 tool call 是交错的
    pub fn push_part(&mut self, part: DisplayPart) {
        match (self.parts.last_mut(), &part) {
            (Some(DisplayPart::Content(existing)), DisplayPart::Content(new))
            | (Some(DisplayPart::Reasoning(existing)), DisplayPart::Reasoning(new)) => {
                existing.push_str(new);
            }
            _ => self.parts.push(part),
        }
    }

    pub fn has_output_content(&self) -> bool {
        self.parts.iter().any(|part| match part {
            DisplayPart::Content(content) => !content.is_empty(),
            _ => false,
        })
    }
}

impl ToolResultView {
    pub fn new(status: ToolStatus, raw_output: String) -> Self {
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

pub fn summarize_tool_call(call: &ToolCall) -> String {
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

/// 把会话历史转换成展示用的消息列表。
pub fn display_history(messages: Vec<ModelMessage>) -> Vec<DisplayMessage> {
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

                // 带工具调用的 assistant 消息属于中间过程，只展示工具调用，不展示正文
                let has_tool_calls = assistant
                    .tool_calls
                    .as_ref()
                    .is_some_and(|calls| !calls.is_empty());

                if !has_tool_calls {
                    if let Some(content) = assistant.content {
                        if !content.is_empty() {
                            current.push_part(DisplayPart::Content(content));
                        }
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
