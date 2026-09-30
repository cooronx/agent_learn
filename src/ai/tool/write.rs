use std::{
    io::ErrorKind::PermissionDenied,
    sync::atomic::{AtomicU64, Ordering},
};

use async_trait::async_trait;
use color_eyre::eyre::eyre;
use schemars::{JsonSchema, schema_for};
use serde::Deserialize;
use std::result::Result::Ok;
use tokio::{fs, io::AsyncWriteExt};

use crate::ai::tool::Tool;

// 用一个原子变量来生成临时文件名，避免冲突
static TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
pub struct WriteTool;

#[derive(Deserialize, JsonSchema)]
pub struct WriteParameters {
    #[schemars(
        length(min = 1),
        description = "File Path. Relative paths resolve from the current working directory"
    )]
    path: String,

    #[schemars(
        description = "Complete UTF-8 content to write. Replaces all existing file content"
    )]
    content: String,
}

#[async_trait]
impl Tool for WriteTool {
    fn name(&self) -> String {
        "write".to_string()
    }

    fn description(&self) -> Option<String> {
        Some(
            "Create a UTF-8 text file or overwrite an existing regular file. \
               Creates missing parent directories. Existing content is fully replaced; \
               read an existing file before overwriting it. Symbolic links are rejected."
                .to_string(),
        )
    }

    fn parameters(&self) -> Option<serde_json::Value> {
        Some(schema_for!(WriteParameters).to_value())
    }

    async fn execute(&self, args: serde_json::Value) -> color_eyre::Result<String> {
        let args: WriteParameters = serde_json::from_value(args)?;

        if args.path.is_empty() {
            return Err(eyre!("path must not be empty"));
        }

        let path = std::env::current_dir()?.join(&args.path);

        let permissions = match fs::symlink_metadata(&path).await {
            Ok(metadata) => {
                if !metadata.file_type().is_file() {
                    return Err(eyre!("target must be a regular file"));
                }
                // 只读的肯定不能写
                if metadata.permissions().readonly() {
                    return Err(eyre!("target is read-only"));
                }
                Some(metadata.permissions())
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
            Err(err) => return Err(err.into()),
        };

        let parent = path
            .parent()
            .ok_or_else(|| eyre!("path must have a parent directory"))?;

        // 直接把文件夹给创建了（如果不存在的话
        fs::create_dir_all(parent).await?;

        let temp_path = parent.join(format!(
            ".agent-write-{}-{}",
            std::process::id(),
            TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed)
        ));

        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .await?;

        let ret = async {
            file.write_all(args.content.as_bytes()).await?;
            if let Some(permission) = permissions {
                file.set_permissions(permission).await?;
            }

            file.sync_all().await?;
            drop(file);

            fs::rename(&temp_path, &path).await
        }
        .await;

        if ret.is_err() {
            let _ = fs::remove_file(&temp_path).await;
        }

        ret?;

        Ok(serde_json::json!({
            "path": path,
            "bytes_written": &args.content.len()
        })
        .to_string())
    }
}
