use std::{
    fs::{File, OpenOptions},
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

use std::io::Write;

use serde::{Deserialize, Serialize};

use chrono::Utc;

use crate::ai::types::ModelMessage;

// 一次对话的元信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    // uuid
    pub id: String,
    pub cwd: String,
    pub model: String,
    // 创建时间（unix毫秒）
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
enum SessionRecord {
    Meta(SessionMeta),
    Message(ModelMessage),
}

pub struct Session {
    path: PathBuf,
    meta: SessionMeta,
    file: File,
}

impl Session {
    /// 新建会话
    pub fn create(
        dir: &Path,
        cwd: &str,
        model: &str,
    ) -> color_eyre::Result<(Self, Vec<ModelMessage>)> {
        let id = uuid::Uuid::new_v4().to_string();
        let path = format!("{}.jsonl", id);
        let path = dir.join(path);

        let meta = SessionMeta {
            id,
            cwd: cwd.to_string(),
            model: model.to_string(),
            created_at: Utc::now().timestamp_millis() as u64,
        };

        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        writeln!(
            file,
            "{}",
            serde_json::to_string(&SessionRecord::Meta(meta.clone()))?
        )?;
        file.flush()?;

        Ok((Self { path, meta, file }, Vec::new()))
    }

    /// 打开一个已经存在的session文件
    pub fn open(path: &Path) -> color_eyre::Result<(Self, Vec<ModelMessage>)> {
        let (meta, messages) = read_session(path)?;
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

    /// 同步记录一条进入session
    pub fn append_message(&mut self, message: ModelMessage) -> color_eyre::Result<()> {
        writeln!(
            self.file,
            "{}",
            serde_json::to_string(&SessionRecord::Message(message))?
        )?;
        self.file.flush()?;
        Ok(())
    }
}

/// 读取Session文件
fn read_session(path: &Path) -> color_eyre::Result<(SessionMeta, Vec<ModelMessage>)> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut meta = None;
    let mut messages = Vec::new();

    for line in reader.lines() {
        let line = line?;

        // 反序列化失败的情况，可能是这个session崩溃了之类的情况
        let Ok(record) = serde_json::from_str::<SessionRecord>(&line) else {
            continue;
        };

        match record {
            SessionRecord::Meta(session_meta) => meta = Some(session_meta),
            SessionRecord::Message(model_message) => messages.push(model_message),
        }
    }

    let meta =
        meta.ok_or_else(|| color_eyre::eyre::eyre!("session file no meta: {}", path.display()))?;

    Ok((meta, messages))
}
