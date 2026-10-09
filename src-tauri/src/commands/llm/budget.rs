//! Context token estimation and error classification helpers (rough pre-check / overflow detection / retryability).

/// Estimate the token count of a single API message (local tiktoken count; falls back to chars/2 on failure).
/// Used to decide when to trigger context compression and the retention budget.
pub(crate) fn estimate_message_tokens(model: &str, msg: &serde_json::Value) -> usize {
    let s = serde_json::to_string(msg).unwrap_or_default();
    crate::utils::token_counter::count_tokens(model, &s).unwrap_or(s.len() / 2)
}

/// CJK-aware character-level rough estimate (integer quarter arithmetic, 1 token = 4 quarters):
/// - ASCII chars: ~4 chars/token (cl100k/o200k average for English)
/// - Non-ASCII (CJK/accented/emoji): ~1.25 tokens/char
///
/// Versus the old bytes/3 heuristic: Chinese text (3 bytes/char ≈ 1.0-1.5 real tokens)
/// moves from up to ~50% underestimation to ±25%; English from ~33% overestimation to ±20%.
/// The rough estimate only decides whether to run exact tiktoken counting — erring high
/// (one extra exact pass) beats erring low (a missed compression trigger, caught only by overflow errors).
pub(crate) fn rough_tokens(s: &str) -> usize {
    let mut quarters = 0usize;
    for ch in s.chars() {
        if ch.is_ascii() {
            quarters += 1; // 4 ASCII chars ≈ 1 token
        } else {
            quarters += 5; // non-ASCII chars ≈ 1.25 tokens
        }
    }
    quarters / 4
}

/// Context token estimation (with a rough pre-check): use the CJK-aware rough estimate first and
/// return it directly when far below budget, avoiding tiktoken-encoding all messages every round; count exactly only near budget.
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

/// Whether an API error is caused by context-length overflow (used for overflow auto-recovery: retry once after compression).
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

/// Whether a tool error is retryable (network-class errors are retryable; permission/parameter errors are not)
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

    /// The Chinese rough estimate must not fall significantly below the exact count (the old
    /// bytes/3 heuristic underestimates CJK by up to ~50%, missing compression triggers). Acceptance: within 2x of exact and above the old heuristic.
    #[test]
    fn rough_estimate_is_cjk_aware() {
        let zh = "这是一段用于验证上下文预算估算的中文文本，包含标点、顿号与常见汉字词汇。";
        let model = "gpt-4"; // cl100k encoder
        let exact =
            crate::utils::token_counter::count_tokens(model, zh).expect("tiktoken 计数失败");
        let rough = rough_tokens(zh);
        assert!(
            rough >= exact / 2 && rough <= exact * 2,
            "中文粗估偏差过大: rough={rough} exact={exact}"
        );
        // Old heuristic for comparison: bytes/3 (3 bytes per CJK char -> 1.0 token/char)
        let old = zh.len() / 3;
        assert!(
            rough > old,
            "CJK 粗估应高于字节/3 启发式: rough={rough} old={old}"
        );
    }

    /// The English rough estimate must likewise stay within 2x of exact (assumes 4 ASCII chars/token).
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

    /// Empty-string and pure-ASCII edge cases.
    #[test]
    fn rough_estimate_edges() {
        assert_eq!(rough_tokens(""), 0);
        assert_eq!(rough_tokens("abcd"), 1); // 4 quarters / 4
        assert_eq!(rough_tokens("abcde"), 1); // 5 quarters / 4 = 1
        assert!(rough_tokens("中") >= 1);
    }
}
