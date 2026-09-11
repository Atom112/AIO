//! 上下文 token 估算与错误分类辅助（粗估预检 / 溢出判定 / 可重试判定）。

/// 估算单条 API 消息的 token 数（tiktoken 本地计数；失败时回退到字符数/2）。
/// 用于上下文压缩触发与保留预算的判定。
pub(crate) fn estimate_message_tokens(model: &str, msg: &serde_json::Value) -> usize {
    let s = serde_json::to_string(msg).unwrap_or_default();
    crate::utils::token_counter::count_tokens(model, &s).unwrap_or(s.len() / 2)
}

/// 上下文 token 估算（带粗估预检）：先用 字符数/3 粗估，远低于预算时直接返回，
/// 避免每轮对全部消息做 tiktoken 编码拖慢 Agent 循环；接近预算时才精确统计。
pub(crate) fn estimate_context_tokens(
    model: &str,
    messages: &[serde_json::Value],
    budget: usize,
) -> usize {
    let rough: usize = messages
        .iter()
        .map(|m| serde_json::to_string(m).map(|s| s.len() / 3).unwrap_or(0))
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
