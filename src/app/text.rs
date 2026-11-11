//! 用到的工具函数

/// 工具输出预览时的横向宽度上限
pub const TOOL_PREVIEW_WIDTH: usize = 100;

/// 从左侧截断字符串，超出长度时保留尾部并加省略号（适用于路径）
pub fn truncate_left(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    // 保留 max-1 个尾部字符，前面加省略号
    let keep = max.saturating_sub(1);
    let tail: String = chars[chars.len() - keep..].iter().collect();
    format!("…{tail}")
}

/// 从右侧截断字符串，超出长度时保留头部并加省略号（适用于标题）
pub fn truncate_right(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let head: String = chars[..keep].iter().collect();
    format!("{head}…")
}

/// 把毫秒时间戳格式化成「刚刚 / N 分钟前 / N 小时前 / N 天前」
pub fn format_relative_time(created_at: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let seconds = now.saturating_sub(created_at) / 1000;

    match seconds {
        0..=59 => "刚刚".to_string(),
        60..=3599 => format!("{} 分钟前", seconds / 60),
        3600..=86_399 => format!("{} 小时前", seconds / 3600),
        _ => format!("{} 天前", seconds / 86_400),
    }
}
