//! Token counting utilities
//!
//! Wraps tiktoken-rs for local token counting:
//! - Token count for a single text
//! - Token estimation for chat message arrays
//! - Automatic model name -> encoder mapping
//!
//! Performance note: tiktoken-rs's cl100k_base()/o200k_base() re-parses the entire
//! BPE vocab (100k+ entries, base64 decode + HashMap build, ~100ms) on each call;
//! encoders must be globally cached or per-message counting stalls the agent loop.

use std::sync::{LazyLock, Mutex};
use tiktoken_rs::CoreBPE;

/// cl100k_base (GPT-4 / GPT-3.5 family): globally cached, parsed only once
static CL100K: LazyLock<Mutex<CoreBPE>> = LazyLock::new(|| {
    Mutex::new(tiktoken_rs::cl100k_base().expect("无法加载 cl100k_base tokenizer"))
});
/// o200k_base (GPT-4o / o1 / o3 family)
static O200K: LazyLock<Mutex<CoreBPE>> =
    LazyLock::new(|| Mutex::new(tiktoken_rs::o200k_base().expect("无法加载 o200k_base tokenizer")));
/// p50k_base (davinci / babbage / curie / ada family)
static P50K: LazyLock<Mutex<CoreBPE>> =
    LazyLock::new(|| Mutex::new(tiktoken_rs::p50k_base().expect("无法加载 p50k_base tokenizer")));

/// Select the encoder by model name (returns a globally cached reference, zero rebuild cost)
fn get_bpe(model: &str) -> &'static Mutex<CoreBPE> {
    // o200k takes priority (gpt-4o also matches the "gpt-4" prefix, so check the o-series first)
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
        // Default to cl100k_base (the most common estimate for GPT-4 / GPT-3.5 / compatible models)
        &CL100K
    }
}

/// Estimate the token count of a single text
pub fn count_tokens(model: &str, text: &str) -> Result<usize, String> {
    let bpe = get_bpe(model);
    let bpe = bpe.lock().map_err(|e| e.to_string())?;
    let tokens = bpe.encode_with_special_tokens(text);
    Ok(tokens.len())
}

// ====== Tauri commands ======

/// Estimate the token count of a text (for live preview in the frontend input box)
pub fn count_tokens_cmd(model: String, text: String) -> Result<usize, String> {
    count_tokens(&model, &text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Performance regression guard: BPE encoders must be globally cached
    /// (re-parsing the vocab on every call stalls the agent loop).
    /// 100 short-text counts should take < 100ms cached; uncached ~10-30s, which trips the lenient 10s limit.
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
