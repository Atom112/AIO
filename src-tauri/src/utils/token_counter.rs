//! Token 计数工具
//!
//! 封装 tiktoken-rs 提供本地 token 计数能力：
//! - 单文本 token 计数
//! - 聊天消息数组 token 估算
//! - 模型名 → 编码器自动映射

use tiktoken_rs::CoreBPE;

/// 根据模型名获取对应的 tiktoken 编码器
fn get_bpe(model: &str) -> Result<CoreBPE, String> {
    // 尝试通过模型名获取（tiktoken-rs 内置支持 OpenAI 模型）
    if let Ok(bpe) = tiktoken_rs::cl100k_base() {
        // cl100k_base 覆盖 GPT-4 / GPT-3.5 系列
        if model.contains("gpt-4") || model.contains("gpt-3.5") || model.contains("text-embedding") {
            return Ok(bpe);
        }
    }
    if let Ok(bpe) = tiktoken_rs::o200k_base() {
        // o200k_base 覆盖 GPT-4o 系列
        if model.contains("gpt-4o") || model.contains("o1") || model.contains("o3") {
            return Ok(bpe);
        }
    }
    if let Ok(bpe) = tiktoken_rs::p50k_base() {
        if model.contains("davinci") || model.contains("babbage") || model.contains("curie") || model.contains("ada") {
            return Ok(bpe);
        }
    }
    // fallback 到 cl100k_base（最常用）
    tiktoken_rs::cl100k_base().map_err(|e| format!("无法加载 tokenizer: {}", e))
}

/// 估算单个文本的 token 数
pub fn count_tokens(model: &str, text: &str) -> Result<usize, String> {
    let bpe = get_bpe(model)?;
    let tokens = bpe.encode_with_special_tokens(text);
    Ok(tokens.len())
}

/// 估算聊天消息数组的 token 数
///
/// 使用 OpenAI 的简化公式：每条消息约 4 tokens 的格式开销 + 内容 tokens
pub fn count_chat_tokens(model: &str, messages: &[serde_json::Value]) -> Result<usize, String> {
    let bpe = get_bpe(model)?;
    let mut total = 0;

    for msg in messages {
        // 每条消息的格式开销（role + 分隔符）
        total += 4;
        if let Some(content) = msg.get("content") {
            let text = match content {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let tokens = bpe.encode_with_special_tokens(&text);
            total += tokens.len();
        }
        // tool_calls 的格式开销
        if let Some(tool_calls) = msg.get("tool_calls") {
            if let Some(arr) = tool_calls.as_array() {
                for tc in arr {
                    total += 4; // 每个 tool_call 格式开销
                    if let Some(func) = tc.get("function") {
                        if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                            total += bpe.encode_with_special_tokens(name).len();
                        }
                        if let Some(args) = func.get("arguments").and_then(|v| v.as_str()) {
                            total += bpe.encode_with_special_tokens(args).len();
                        }
                    }
                }
            }
        }
    }

    // 回复 priming（约 3 tokens）
    total += 3;

    Ok(total)
}

// ====== Tauri 命令 ======

/// 估算文本的 token 数（供前端输入框实时预览）
pub fn count_tokens_cmd(model: String, text: String) -> Result<usize, String> {
    count_tokens(&model, &text)
}
