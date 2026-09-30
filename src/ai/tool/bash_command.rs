use std::{path::PathBuf, process::Stdio, time::Duration};

use async_trait::async_trait;
use color_eyre::eyre::eyre;
use schemars::{JsonSchema, schema_for};
use serde::Deserialize;
use tokio::process::Command;

use crate::ai::tool::Tool;

#[derive(Default)]
pub struct BashCommandTool;

#[derive(Deserialize, JsonSchema)]
pub struct BashCommandParameters {
    #[schemars(length(min = 1), description = "The Bash command to execute")]
    command: String,
    #[schemars(
        description = "Working directory. Relative paths are resolved from the startup directory. Default to the startup directory"
    )]
    cwd: Option<String>,
    #[schemars(
        range(min = 1, max = 300000),
        description = "Maximum execution time in milliseconds. Defaults to 300,000."
    )]
    timeout_ms: Option<u64>,
    #[schemars(
        range(min = 1, max = 1000000),
        description = "Maximum characters for stdout and stderr. Defaults to 20,000"
    )]
    max_output_chars: Option<usize>,
}

#[async_trait]
impl Tool for BashCommandTool {
    fn name(&self) -> String {
        "bash_command".to_string()
    }

    fn description(&self) -> Option<String> {
        Some(
            "Execute a Bash command. The command runs in the requested working directory \
               and returns exit code, stdout, and stderr. Commands are subject to timeout \
               and output limits."
                .to_string(),
        )
    }

    fn parameters(&self) -> Option<serde_json::Value> {
        Some(schema_for!(BashCommandParameters).to_value())
    }

    async fn execute(&self, args: serde_json::Value) -> color_eyre::Result<String> {
        let args: BashCommandParameters = serde_json::from_value(args)?;

        if args.command.trim().is_empty() {
            return Err(eyre!("command must not be empty"));
        }

        let timeout_ms = args.timeout_ms.unwrap_or(300_000);
        if timeout_ms < 1 || timeout_ms > 300_000 {
            return Err(eyre!("timeout_ms must between 1 and 300000"));
        }

        let max_output_chars = args.max_output_chars.unwrap_or(20_000);
        if max_output_chars < 1 || max_output_chars > 1_000_000 {
            return Err(eyre!("max_output_chars must between 1 and 1_000_000"));
        }

        // 确定命令的工作目录
        let cwd = std::env::current_dir()?;
        let cwd = match args.cwd {
            Some(path) => cwd.join(path),
            None => cwd,
        };

        // 创建异步子进程来执行命令
        let mut process = Command::new("bash");
        process
            .arg("--noprofile")
            .arg("-norc")
            .arg("-o")
            // 管道中任意一个命令错误，直接返回这个错误码
            .arg("pipefail")
            .arg("-c")
            .arg(&args.command)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let output = tokio::time::timeout(Duration::from_millis(timeout_ms), process.output())
            .await
            .map_err(|_| eyre!("command timed out after {timeout_ms} ms"))??;

        let truncate = |bytes: &[u8]| -> (String, bool) {
            let text = String::from_utf8_lossy(bytes);
            match text.char_indices().nth(max_output_chars) {
                Some((byte_index, _)) => (text[..byte_index].to_owned(), true),
                None => (text.into_owned(), false),
            }
        };

        let (stdout, is_stdout_truncated) = truncate(&output.stdout);
        let (stderr, is_stderr_truncated) = truncate(&output.stderr);

        Ok(serde_json::json!({
            "success": output.status.success(),
            "exit_code": output.status.code(),
            "stdout": stdout,
            "stderr": stderr,
            "is_stdout_truncated": is_stdout_truncated,
            "is_stderr_truncated": is_stderr_truncated,
        })
        .to_string())
    }
}
