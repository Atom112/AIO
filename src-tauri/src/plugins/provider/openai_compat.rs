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
            let trimmed = api_url.trim_end_matches('/');
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
