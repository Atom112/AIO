//! Text truncation helpers — single source of truth repo-wide.
//!
//! Semantics are baked into the function names, with two length units:
//! - `chars`: by Unicode character count (for display/prompt text)
//! - `bytes`: byte cap landing on a char boundary (for tool results/command output, controlling token and transfer size)

/// Maximum bytes for tool result content (20k chars ≈ 5-10k tokens).
pub const TOOL_RESULT_MAX_BYTES: usize = 20_000;

/// Head truncation (by chars): keep the first `max_chars` characters, append "…" when over.
/// Suitable for short UI-display summaries.
pub fn truncate_chars_head(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max_chars).collect::<String>())
    }
}

/// Head truncation (by bytes, auto-landing on a char boundary): appends a "[!]" truncation notice when over.
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

/// Head-tail truncation (by chars): keep the first 60% and last 40%, joined by an ellipsis marker.
/// For assembled contexts (e.g. workflow step context) where both ends carry key information.
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

/// Head-tail truncation (by bytes, front 60% + back 40%) — default truncation strategy for tool results.
///
/// Keep both ends instead of head-only: `read_file` already preserves a file's head and tail;
/// truncating to head-only here would cut the tail a second time, hiding key content from the model.
pub fn truncate_tool_result(s: &str) -> String {
    // Replace empty results with a very short placeholder to keep junk out of the context
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

/// Command/subprocess output truncation (&[u8] → lossy String): appends byte-count info when over the limit.
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
        // Multi-byte characters: no panic, no mid-character cut
        assert_eq!(truncate_chars_head("中文测试", 2), "中文…");
    }

    #[test]
    fn bytes_head_stays_on_char_boundary() {
        let s = "中文测试";
        let out = truncate_bytes_head(s, 5); // 5 is not a char boundary
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
