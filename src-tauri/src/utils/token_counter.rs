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
        if model.contains("gpt-4") || model.contains("gpt-3.5") || model.contains("text-embedding")
        {
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
        if model.contains("davinci")
            || model.contains("babbage")
            || model.contains("curie")
            || model.contains("ada")
        {
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

// ====== Tauri 命令 ======

/// 估算文本的 token 数（供前端输入框实时预览）
pub fn count_tokens_cmd(model: String, text: String) -> Result<usize, String> {
    count_tokens(&model, &text)
}
