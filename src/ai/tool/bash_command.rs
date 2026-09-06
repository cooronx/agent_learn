use std::path::PathBuf;

use async_trait::async_trait;
use color_eyre::eyre::eyre;

use crate::ai::tool::Tool;




pub struct BashCommandTool {
    default_cwd: PathBuf,
}

impl Default for BashCommandTool {
    fn default() -> Self {
        Self { default_cwd: std::env::current_dir().unwrap_or_else(|_|PathBuf::from(".")) }
    }
}

#[async_trait]
impl Tool for BashCommandTool {
    fn name(&self) -> String {
        "bash_command".to_string()
    }

    fn description(&self) -> Option<String>  {
        Some(
              "Execute a Bash command. The command runs in the requested working directory \
               and returns exit code, stdout, and stderr. Commands are subject to timeout \
               and output limits."
                  .to_string(),
        )
    }

    fn parameters(&self) -> Option<serde_json::Value>  {
        Some(serde_json::json!({
              "type": "object",
              "properties": {
                  "command": {
                      "type": "string",
                      "description": "The Bash command to execute"
                  },
                  "cwd": {
                      "type": "string",
                      "description": "Working directory. Relative paths are resolved from the startup directory."
                  },
                  "timeout_ms": {
                      "type": "integer",
                      "minimum": 1,
                      "maximum": 300000,
                      "description": "Maximum execution time in milliseconds. Defaults to 30,000."
                  },
                  "max_output_chars": {
                      "type": "integer",
                      "minimum": 1,
                      "maximum": 1000000,
                      "description": "Maximum stdout and stderr characters returned per stream. Defaults to 20,000."
                  }
              },
              "required": ["command"],
              "additionalProperties": false,
          }))
    }


    async fn execute(&self, args: serde_json::Value) -> color_eyre::Result<String> {

        let command = args
            .get("command")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(||eyre!("missing parameter: command"))?;

        let cwd = match args.get("cwd").and_then(serde_json::Value::as_str) {
            Some(path) => {
                let path = std::path::Path::new(path);

                // 如果是绝对地址，就直接用这个地址
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    // 不然就拼接到现在的地址后面
                    self.default_cwd.join(path)
                }
            },
            None => self.default_cwd.clone(),
        };

        let timeout_ms = args
            .get("timeout_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(30_000);

        if timeout_ms == 0 || timeout_ms > 300_000 {
            return Err(eyre!("timeout_ms must between 1 and 300000"))
        }
        

        Ok(String::default())
    }
    
}