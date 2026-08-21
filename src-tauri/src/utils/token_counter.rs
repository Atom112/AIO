//! Token 计数工具
//!
//! 封装 tiktoken-rs 提供本地 token 计数能力：
//! - 单文本 token 计数
//! - 聊天消息数组 token 估算
//! - 模型名 → 编码器自动映射
//!
//! 性能说明：tiktoken-rs 的 cl100k_base()/o200k_base() 每次调用都会重新解析
//! 整个 BPE 词表（10 万+ 条、base64 解码 + HashMap 构建，约百毫秒级），
//! 必须全局缓存，否则 Agent 循环中逐消息调用会严重拖慢（曾导致子代理工具调用卡顿数分钟）。

use std::sync::{LazyLock, Mutex};
use tiktoken_rs::CoreBPE;

/// cl100k_base（GPT-4 / GPT-3.5 系列）——全局缓存，只解析一次
static CL100K: LazyLock<Mutex<CoreBPE>> = LazyLock::new(|| {
    Mutex::new(tiktoken_rs::cl100k_base().expect("无法加载 cl100k_base tokenizer"))
});
/// o200k_base（GPT-4o / o1 / o3 系列）
static O200K: LazyLock<Mutex<CoreBPE>> =
    LazyLock::new(|| Mutex::new(tiktoken_rs::o200k_base().expect("无法加载 o200k_base tokenizer")));
/// p50k_base（davinci / babbage / curie / ada 系列）
static P50K: LazyLock<Mutex<CoreBPE>> =
    LazyLock::new(|| Mutex::new(tiktoken_rs::p50k_base().expect("无法加载 p50k_base tokenizer")));

/// 根据模型名选择编码器（返回全局缓存引用，零重建开销）
fn get_bpe(model: &str) -> &'static Mutex<CoreBPE> {
    // o200k 优先（gpt-4o 同时匹配 "gpt-4" 前缀，必须先判 o 系列）
    if model.contains("gpt-4o")
        || model.contains("o1")
        || model.contains("o3")
        || model.contains("o4")
        || model.contains("gpt-5")
        || model.contains("claude")
    {
        &O200K
    } else if model.contains("davinci")
        || model.contains("babbage")
        || model.contains("curie")
        || model.contains("ada")
    {
        &P50K
    } else {
        // 默认 cl100k_base（GPT-4 / GPT-3.5 / 兼容模型的最常用估算）
        &CL100K
    }
}

/// 估算单个文本的 token 数
pub fn count_tokens(model: &str, text: &str) -> Result<usize, String> {
    let bpe = get_bpe(model);
    let bpe = bpe.lock().map_err(|e| e.to_string())?;
    let tokens = bpe.encode_with_special_tokens(text);
    Ok(tokens.len())
}

// ====== Tauri 命令 ======

/// 估算文本的 token 数（供前端输入框实时预览）
pub fn count_tokens_cmd(model: String, text: String) -> Result<usize, String> {
    count_tokens(&model, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 性能回归守卫：BPE 编码器必须全局缓存（每次调用重新解析词表会导致 Agent 循环卡顿）。
    /// 100 次短文本计数在缓存下应 < 100ms；未缓存时约 10-30 秒，必然触发 10s 宽松上限。
    #[test]
    fn count_tokens_is_cached_and_fast() {
        let text = "你好，这是一个用于 token 计数的测试文本 hello world 12345";
        let first = count_tokens("gpt-4", text).unwrap();
        let start = std::time::Instant::now();
        let mut last = 0;
        for _ in 0..100 {
            last = count_tokens("gpt-4", text).unwrap();
        }
        let elapsed = start.elapsed();
        assert_eq!(first, last, "重复计数结果应一致");
        assert!(
            elapsed.as_secs() < 10,
            "count_tokens 100 次耗时 {elapsed:?}，疑似 BPE 未缓存"
        );
    }
}
