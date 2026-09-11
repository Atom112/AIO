//! 文本截断辅助 —— 全仓唯一来源。
//!
//! 历史上同名 "truncate" 有 6 份实现、3 种互不相同的语义（按字节 / 按字符 /
//! 头尾保留），调用方极易选错。此处把语义写进函数名，并提供两种长度单位：
//! - `chars`：按 Unicode 字符数（面向展示/提示文本）
//! - `bytes`：按字节上限并落在字符边界（面向工具返回/命令输出，控制 token 与传输量）

/// 工具返回内容的最大字节数（20k 字符 ≈ 5-10k tokens）。
pub const TOOL_RESULT_MAX_BYTES: usize = 20_000;

/// 头部截断（按字符）：保留前 `max_chars` 个字符，超限追加 "…"。
/// 适用于 UI 展示用的短摘要。
pub fn truncate_chars_head(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max_chars).collect::<String>())
    }
}

/// 头部截断（按字节，自动落在字符边界）：超限追加 "[!] 已截断" 提示。
pub fn truncate_bytes_head(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_string();
    }
    let safe_end = s.floor_char_boundary(limit);
    format!(
        "{}\n\n[!] 输出过长已截断 ({safe_end}B / {}B)",
        &s[..safe_end],
        s.len()
    )
}

/// 头尾截断（按字符）：保留前 60% 与后 40%，中间以省略标记连接。
/// 用于工作流步骤上下文等"需要保留首尾关键信息"的拼接场景。
pub fn truncate_chars_head_tail(s: &str, max_chars: usize) -> String {
    let total = s.chars().count();
    if total <= max_chars {
        return s.to_string();
    }
    let head = (max_chars * 6) / 10;
    let tail = max_chars - head;
    let head_str: String = s.chars().take(head).collect();
    let tail_str: String = s.chars().skip(total - tail).collect();
    format!(
        "{}\n... [已省略 {} 字符] ...\n{}",
        head_str,
        total - head - tail,
        tail_str
    )
}

/// 头尾截断（按字节，前 60% + 后 40%）—— 工具返回内容的默认截断策略。
///
/// 头尾保留而非只留头部：`read_file` 已保留文件首尾，若这里只留头部会把
/// 尾部信息二次砍掉，导致模型探索时看不到关键内容。
pub fn truncate_tool_result(s: &str) -> String {
    // 空结果统一替换为极短占位，避免垃圾进上下文
    if s.trim().is_empty() {
        return "[无结果]".to_string();
    }
    if s.len() <= TOOL_RESULT_MAX_BYTES {
        return s.to_string();
    }
    let head = (TOOL_RESULT_MAX_BYTES * 6) / 10;
    let tail = TOOL_RESULT_MAX_BYTES - head;
    let head_end = s.floor_char_boundary(head.min(s.len()));
    let tail_start = s.floor_char_boundary(s.len().saturating_sub(tail));
    if tail_start > head_end {
        format!(
            "{}\n\n... [已截断: 共{}字符，保留首尾] ...\n\n{}",
            &s[..head_end],
            s.len(),
            &s[tail_start..]
        )
    } else {
        let safe_end = s.floor_char_boundary(TOOL_RESULT_MAX_BYTES.min(s.len()));
        format!("{}\n\n... [已截断: 共{}字符]", &s[..safe_end], s.len())
    }
}

/// 命令/子进程输出截断（&[u8] → lossy String）：超限追加字节数说明。
pub fn truncate_bytes_lossy(output: &[u8], limit: usize) -> String {
    if output.len() <= limit {
        String::from_utf8_lossy(output).into_owned()
    } else {
        format!(
            "{}\n[!] 输出已截断 ({}B / {}B)",
            String::from_utf8_lossy(&output[..limit]),
            limit,
            output.len()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chars_head_keeps_head_only() {
        assert_eq!(truncate_chars_head("abcdef", 3), "abc…");
        assert_eq!(truncate_chars_head("abc", 3), "abc");
        // 多字节字符不 panic、不切坏
        assert_eq!(truncate_chars_head("中文测试", 2), "中文…");
    }

    #[test]
    fn bytes_head_stays_on_char_boundary() {
        let s = "中文测试";
        let out = truncate_bytes_head(s, 5); // 5 不是字符边界
        assert!(out.starts_with("中"), "输出: {out}");
        assert!(out.contains("[!]"));
    }

    #[test]
    fn head_tail_keeps_both_ends() {
        let s: String = (0..100)
            .map(|i| char::from(b'a' + (i % 26) as u8))
            .collect();
        let out = truncate_chars_head_tail(&s, 20);
        assert!(out.starts_with(&s[..12]), "头 12 字符应保留");
        assert!(out.ends_with(&s[92..]), "尾 8 字符应保留");
        assert!(out.contains("已省略"));
    }

    #[test]
    fn tool_result_handles_empty_and_large() {
        assert_eq!(truncate_tool_result("   "), "[无结果]");
        let small = "ok";
        assert_eq!(truncate_tool_result(small), "ok");
        let big = "x".repeat(TOOL_RESULT_MAX_BYTES + 100);
        let out = truncate_tool_result(&big);
        assert!(out.contains("已截断"));
        assert!(out.len() < big.len());
    }

    #[test]
    fn bytes_lossy_truncates() {
        let data = vec![b'a'; 100];
        assert!(truncate_bytes_lossy(&data, 10).contains("[!]"));
        assert_eq!(truncate_bytes_lossy(b"hi", 10), "hi");
    }
}
