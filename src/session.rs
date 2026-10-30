use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use chrono::Utc;
use color_eyre::{
    Result,
    eyre::{WrapErr, eyre},
};
use serde::{Deserialize, Serialize};

use crate::ai::types::ModelMessage;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub cwd: String,
    pub title: String,
    pub model: String,
    /// Unix 毫秒。
    pub created_at: u64,
}

/// 一份会话对应 <id>.meta.json 和 <id>.jsonl。
pub struct Session {
    path: PathBuf,
    meta: SessionMeta,
    file: File,
}

impl Session {
    pub fn create(dir: &Path, cwd: &str, title: &str, model: &str) -> Result<Self> {
        fs::create_dir_all(dir)?;
        let meta = SessionMeta {
            id: uuid::Uuid::new_v4().to_string(),
            cwd: cwd.to_string(),
            title: title.to_string(),
            model: model.to_string(),
            created_at: Utc::now().timestamp_millis() as u64,
        };
        let path = dir.join(format!("{}.jsonl", meta.id));
        let file = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(&path)?;
        file.sync_all()?;
        if let Err(error) = write_meta(&path.with_extension("meta.json"), &meta) {
            drop(file);
            let _ = fs::remove_file(&path);
            return Err(error);
        }
        Ok(Self { path, meta, file })
    }

    /// path 是消息文件 <id>.jsonl
    pub fn open(path: &Path) -> Result<(Self, Vec<ModelMessage>)> {
        if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
            return Err(eyre!("expected a .jsonl session path: {}", path.display()));
        }
        let meta = read_meta(path)?;
        let contents = fs::read_to_string(path)?;

        let messages = contents
            .lines()
            .enumerate()
            .map(|(index, line)| {
                serde_json::from_str(line).wrap_err_with(|| {
                    format!("invalid session message {}:{}", path.display(), index + 1)
                })
            })
            .collect::<Result<Vec<ModelMessage>>>()?;
        let file = OpenOptions::new().append(true).open(path)?;
        Ok((
            Self {
                path: path.to_path_buf(),
                meta,
                file,
            },
            messages,
        ))
    }

    pub fn meta(&self) -> &SessionMeta {
        &self.meta
    }

    pub fn set_title(&mut self, title: &str) -> Result<()> {
        let mut meta = self.meta.clone();
        meta.title = title.to_string();
        write_meta(&self.path.with_extension("meta.json"), &meta)?;
        self.meta = meta;
        Ok(())
    }

    pub fn append_message(&mut self, message: &ModelMessage) -> Result<()> {
        writeln!(self.file, "{}", serde_json::to_string(message)?)?;
        self.file.sync_all()?;
        Ok(())
    }

    /// 列表只读取元信息，按创建时间倒序排列
    pub fn list(dir: &Path) -> Result<Vec<SessionMeta>> {
        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut sessions = Vec::new();
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let Some(id) = name
                .to_str()
                .and_then(|name| name.strip_suffix(".meta.json"))
            else {
                continue;
            };
            let path = dir.join(format!("{id}.jsonl"));
            if !path.is_file() {
                return Err(eyre!("session message file missing: {}", path.display()));
            }
            sessions.push(read_meta(&path)?);
        }
        sessions.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(sessions)
    }

    pub fn session_dir() -> Result<PathBuf> {
        Ok(PathBuf::from(std::env::var("HOME")?).join(".cxagent"))
    }
}

fn read_meta(message_path: &Path) -> Result<SessionMeta> {
    let path = message_path.with_extension("meta.json");
    let meta: SessionMeta = serde_json::from_reader(
        File::open(&path)
            .wrap_err_with(|| format!("cannot open session metadata: {}", path.display()))?,
    )
    .wrap_err_with(|| format!("invalid session metadata: {}", path.display()))?;
    Ok(meta)
}

fn write_meta(path: &Path, meta: &SessionMeta) -> Result<()> {
    fs::write(path, serde_json::to_vec_pretty(meta)?)?;
    Ok(())
}
