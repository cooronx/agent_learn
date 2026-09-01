use async_trait::async_trait;
use color_eyre::{Result, eyre::eyre};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::ai::{tool::Tool, types::ToolDefinition};

pub struct ReadFileTool {
    pub max_lines: i32,
}

impl std::default::Default for ReadFileTool {
    fn default() -> Self {
        Self { max_lines: 1000 }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> String {
        "read_file".to_string()
    }

    fn description(&self) -> Option<String> {
        Some(format!("Read the contents of a file. Supports text files.
        For text files, output is truncated to {} lines. 
        Use offset/limit for large files. When you need the full file, continue with offset until complete.",self.max_lines))
    }

    fn parameters(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The path of the file, e.g. /home/cooronx/test.sh"
                },
                "lines": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "How many lines to read, truncated at max_lines"
                }
            },
            "required": ["path"]
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| eyre!("missing parameter: path"))?;
        let max_lines =
            usize::try_from(self.max_lines).map_err(|_| eyre!("max_lines cannot be negative"))?;
        let requested_lines = match args.get("lines") {
            Some(value) => value
                .as_u64()
                .ok_or_else(|| eyre!("parameter lines must be a positive integer"))?
                .try_into()?,
            None => max_lines,
        };
        let line_limit = requested_lines.min(max_lines);

        let fd = tokio::fs::File::open(path).await?;
        let mut lines = BufReader::new(fd).lines();
        let mut content = String::new();

        for line_number in 0..line_limit {
            let Some(line) = lines.next_line().await? else {
                break;
            };
            if line_number > 0 {
                content.push('\n');
            }
            content.push_str(&line);
        }

        Ok(content)
    }
}

impl Into<ToolDefinition> for ReadFileTool {
    fn into(self) -> ToolDefinition {
        ToolDefinition {
            name: self.name(),
            description: self.description(),
            parameters: self.parameters(),
        }
    }
}
