use async_trait::async_trait;
use color_eyre::{Result, eyre::eyre};
use tokio::io::{AsyncBufReadExt, BufReader};

use crate::ai::{tool::Tool, types::ToolDefinition};


/// 读取文件（只支持文本文件）
/// 可以限制一次最多读取多少行
/// 支持偏移读取（还未实现）
pub struct ReadFileTool {
    // 从第几行开始读取，默认下标为1
    pub offset: u32,
    pub limit: u32,
}

impl std::default::Default for ReadFileTool {
    fn default() -> Self {
        Self { offset:0 ,limit: 1000 }
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
        Use offset/limit for large files. When you need the full file, continue with offset until complete.",self.limit))
    }

    fn parameters(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The path of the file, e.g. /home/cooronx/test.sh"
                },
                "offset": {
                    "type": "integer",
                    "minimum": 0,
                    "description": "Which line to start read, the index starts at 0"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1000,
                    "description": "Maximum rows returned per request. Defaults to 1,000."
                }
            },
            "required": ["path","offset"]
        }))
    }

    async fn execute(&self, args: serde_json::Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or_else(|| eyre!("missing parameter: path"))?;
        let offset = args["offset"]
            .as_u64()
            .ok_or_else(||eyre!("missing parameter: offset"))?;
        let max_lines =
            usize::try_from(self.limit).map_err(|_| eyre!("max_lines cannot be negative"))?;
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

        // 先提前跳过offset
        for _ in 0..offset {
            let _ = lines.next_line().await?;
        }

        // 在这里正式开始读取
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

impl From<ToolDefinition> for ReadFileTool {
    fn from(_: ToolDefinition) -> Self {
        Self::default()
    }
}
