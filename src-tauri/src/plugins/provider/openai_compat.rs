//! OpenAI-compatible provider plugin (fallback)
//!
//! Covers: OpenAI / DeepSeek / Groq / Mistral / xAI / Cohere / OpenRouter / custom OpenAI-compatible
//!
//! Endpoint spec:
//! - URL: smartly appends `/v1/models` or `/models`
//! - Auth: `Authorization: Bearer <key>`
//! - Response: `{data: [{id, owned_by}]}`
//!
//! Must be registered last - it is the fallback plugin.

use reqwest::RequestBuilder;

use super::ProviderPlugin;
use crate::core::models::LiveModel;

pub struct OpenAICompatProvider;

impl ProviderPlugin for OpenAICompatProvider {
    fn identifier(&self) -> &'static str {
        "openai_compat"
    }

    fn matches(&self, _api_url: &str) -> bool {
        // Never matches on its own; serves only as the manager's fallback
        false
    }

    fn models_url(&self, api_url: &str) -> String {
        if api_url.ends_with("/models") {
            api_url.to_string()
        } else {
            // A base carrying the full chat path (catalog reality, e.g. bailing's
            // https://api.tbox.cn/api/llm/v1/chat/completions) must not get
            // /v1/models appended AFTER the chat suffix — strip it first, then
            // apply the /v1 rule (chat_completions_url strips the same suffix).
            let trimmed = api_url.trim_end_matches('/');
            let trimmed = trimmed.strip_suffix("/chat/completions").unwrap_or(trimmed);
            if trimmed.ends_with("/v1") || trimmed.ends_with("/v1beta") {
                format!("{}/models", trimmed)
            } else {
                format!("{}/v1/models", trimmed)
            }
        }
    }

    fn apply_auth(&self, req: RequestBuilder, api_key: &str) -> RequestBuilder {
        if api_key.is_empty() {
            req
        } else {
            req.bearer_auth(api_key)
        }
    }

    fn parse_models(&self, body: &serde_json::Value) -> Vec<LiveModel> {
        let arr = body
            .get("data")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        arr.iter()
            .filter_map(|m| {
                let id = m.get("id").and_then(|x| x.as_str())?.to_string();
                let owned = m
                    .get("owned_by")
                    .and_then(|x| x.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                // OpenAI `created` is Unix seconds; convert to YYYY-MM-DD
                let released_at = m
                    .get("created")
                    .and_then(|x| x.as_i64())
                    .and_then(crate::plugins::provider::unix_to_ymd);
                Some(LiveModel {
                    id,
                    owned_by: owned,
                    display_name: None,
                    released_at,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Models URL building: the /v1 rule for bare bases, /models passthrough,
    /// and the full-chat-path base (catalog's bailing shape) must strip the
    /// chat suffix before composing — not append /v1/models after it.
    #[test]
    fn models_url_composes_for_bare_and_full_chat_path_bases() {
        let p = OpenAICompatProvider;
        // /v1 bases: {base}/models
        assert_eq!(
            p.models_url("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/models"
        );
        // Bare host: {base}/v1/models
        assert_eq!(
            p.models_url("https://api.deepseek.com"),
            "https://api.deepseek.com/v1/models"
        );
        // Already a models URL: unchanged
        assert_eq!(
            p.models_url("https://x.example.com/v1/models"),
            "https://x.example.com/v1/models"
        );
        // Full chat-path base (bailing): suffix stripped, then the /v1 rule
        assert_eq!(
            p.models_url("https://api.tbox.cn/api/llm/v1/chat/completions"),
            "https://api.tbox.cn/api/llm/v1/models"
        );
        // Chat path without /v1: fallback append, still after stripping
        assert_eq!(
            p.models_url("https://x.example.com/api/chat/completions"),
            "https://x.example.com/api/v1/models"
        );
    }

    /// Chat URL building mirrors the same suffix handling (regression guard for
    /// the pairing with models_url above).
    #[test]
    fn chat_url_round_trips_full_chat_path_base() {
        let p = OpenAICompatProvider;
        assert_eq!(
            p.chat_completions_url("https://api.tbox.cn/api/llm/v1/chat/completions"),
            "https://api.tbox.cn/api/llm/v1/chat/completions"
        );
        assert_eq!(
            p.chat_completions_url("https://api.deepseek.com"),
            "https://api.deepseek.com/chat/completions"
        );
    }
}
