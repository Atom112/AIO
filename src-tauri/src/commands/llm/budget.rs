//! 上下文 token 估算与错误分类辅助（粗估预检 / 溢出判定 / 可重试判定）。

/// 估算单条 API 消息的 token 数（tiktoken 本地计数；失败时回退到字符数/2）。
/// 用于上下文压缩触发与保留预算的判定。
pub(crate) fn estimate_message_tokens(model: &str, msg: &serde_json::Value) -> usize {
    let s = serde_json::to_string(msg).unwrap_or_default();
    crate::utils::token_counter::count_tokens(model, &s).unwrap_or(s.len() / 2)
}

/// CJK 感知的字符级粗估（整数四分位运算，1 token = 4 quarters）：
/// - ASCII 字符：约 4 字符/token（cl100k/o200k 对英文的平均值）
/// - 非 ASCII（CJK/重音字符/emoji）：约 1.25 token/字符
///
/// 相比旧的 字节len/3 启发式：中文场景（3 字节/字符 ≈ 1.0-1.5 真实 token）
/// 从最多低估 ~50% 收敛到 ±25%；英文从高估 ~33% 收敛到 ±20%。粗估只用于
/// 决定是否运行精确 tiktoken 统计——宁可略高（多跑一次精确统计）也不要
/// 低估（漏掉触发压缩的时机，只能靠溢出报错兜底）。
pub(crate) fn rough_tokens(s: &str) -> usize {
    let mut quarters = 0usize;
    for ch in s.chars() {
        if ch.is_ascii() {
            quarters += 1; // 4 ASCII 字符 ≈ 1 token
        } else {
            quarters += 5; // 非 ASCII 字符 ≈ 1.25 token
        }
    }
    quarters / 4
}

/// 上下文 token 估算（带粗估预检）：先用 CJK 感知粗估，远低于预算时直接返回，
/// 避免每轮对全部消息做 tiktoken 编码拖慢 Agent 循环；接近预算时才精确统计。
pub(crate) fn estimate_context_tokens(
    model: &str,
    messages: &[serde_json::Value],
    budget: usize,
) -> usize {
    let rough: usize = messages
        .iter()
        .map(|m| {
            serde_json::to_string(m)
                .map(|s| rough_tokens(&s))
                .unwrap_or(0)
        })
        .sum();
    if rough < budget.saturating_sub(10_000) {
        return rough;
    }
    messages
        .iter()
        .map(|m| estimate_message_tokens(model, m))
        .sum()
}

/// 判断 API 错误是否由上下文超限导致（用于溢出自动恢复：压缩后重试一次）。
pub(crate) fn is_context_overflow_error(e: &str) -> bool {
    let e = e.to_lowercase();
    [
        "context length",
        "context_length",
        "context window",
        "maximum context",
        "max context",
        "token limit",
        "too many tokens",
        "tokens exceeded",
        "input is too long",
        "prompt is too long",
        "exceeds the maximum",
        "ctx_len",
        "requested tokens",
        "maximum input tokens",
    ]
    .iter()
    .any(|k| e.contains(k))
}

/// 判断工具错误是否可重试（网络类错误可重试，权限/参数错误不可重试）
pub(crate) fn is_retryable_error(e: &str) -> bool {
    let lower = e.to_lowercase();
    lower.contains("timeout")
        || lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
        || lower.contains("eof")
        || lower.contains("not connected")
        || lower.contains("tls")
        || lower.contains("dns")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 中文粗估不得显著低于精确值（旧 字节/3 启发式对 CJK 最多低估 ~50%，
    /// 会漏掉触发压缩的时机）。验收：偏差在精确值的 2 倍以内且高于旧启发式。
    #[test]
    fn rough_estimate_is_cjk_aware() {
        let zh = "这是一段用于验证上下文预算估算的中文文本，包含标点、顿号与常见汉字词汇。";
        let model = "gpt-4"; // cl100k 编码器
        let exact =
            crate::utils::token_counter::count_tokens(model, zh).expect("tiktoken 计数失败");
        let rough = rough_tokens(zh);
        assert!(
            rough >= exact / 2 && rough <= exact * 2,
            "中文粗估偏差过大: rough={rough} exact={exact}"
        );
        // 旧启发式对照：字节/3（每 CJK 字符 3 字节 → 1.0 token/字符）
        let old = zh.len() / 3;
        assert!(
            rough > old,
            "CJK 粗估应高于字节/3 启发式: rough={rough} old={old}"
        );
    }

    /// 英文粗估同样须落在精确值 2 倍以内（ASCII 4 字符/token 假设）。
    #[test]
    fn rough_estimate_english_within_bounds() {
        let en = "The quick brown fox jumps over the lazy dog while the compiler emits warnings. ";
        let model = "gpt-4";
        let exact =
            crate::utils::token_counter::count_tokens(model, en).expect("tiktoken 计数失败");
        let rough = rough_tokens(en);
        assert!(
            rough >= exact / 2 && rough <= exact * 2,
            "英文粗估偏差过大: rough={rough} exact={exact}"
        );
    }

    /// 空串与纯 ASCII 边界。
    #[test]
    fn rough_estimate_edges() {
        assert_eq!(rough_tokens(""), 0);
        assert_eq!(rough_tokens("abcd"), 1); // 4 quarters / 4
        assert_eq!(rough_tokens("abcde"), 1); // 5 quarters / 4 = 1
        assert!(rough_tokens("中") >= 1);
    }
}
