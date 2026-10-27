// 一次对话的元信息
pub struct SessionMeta {
    // uuid
    pub id: String,
    pub cwd: String,
    pub model: String,
    // 创建时间（unix毫秒）
    pub create_at: u64,
}
