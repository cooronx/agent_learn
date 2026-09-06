use async_trait::async_trait;
use color_eyre::{Result, eyre::eyre};
use schemars::{JsonSchema, schema_for};
use serde::Deserialize;
use tokio::{io::{AsyncBufReadExt, BufReader}};

use crate::ai::{tool::Tool, types::ToolDefinition};


/// 读取文件（只支持文本文件）
/// 可以限制一次最多读取多少行
/// 支持偏移读取（还未实现）
pub struct ReadFileTool {
    pub limit: u64,
}


#[derive(JsonSchema,Deserialize)]
pub struct ReadFileToolParameters {
    #[schemars(description = "The path of the file, e.g. /home/cooronx/test.sh")]
    path: String,
    #[schemars(range(min = 0), description = "Which line to start read, the index starts at 0 (default 0)")]
    offset: Option<u64>,
    #[schemars(range(min = 1), description = "Maximum rows returned per request. Defaults to 1,000")]
    limit: Option<u64>,
}

impl std::default::Default for ReadFileTool {
    fn default() -> Self {
        Self { limit: 1000 }
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
        Some(schema_for!(ReadFileToolParameters).to_value())
    }

    async fn execute(&self, args: serde_json::Value) -> Result<String> {

        let paras:ReadFileToolParameters = serde_json::from_value(args)?;

        let path = paras.path;

        // 从第几行开始读取，默认下标为0
        let offset = match paras.offset {
            Some(value) => value,
            None => 0u64,
        };

        let requested_lines = match paras.limit {
            Some(value) => value,
            None => self.limit,
        };
        let line_limit = requested_lines.min(self.limit);

        let fd = tokio::fs::File::open(path).await?;
        let mut lines = BufReader::new(fd).lines();
        let mut content = String::new();

        // 先提前跳过offset
        for _ in 0..offset {
            let Some(_) = lines.next_line().await? else {
                break;
            };
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