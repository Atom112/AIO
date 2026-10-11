//! Ollama provider plugin (local inference)
//!
//! Endpoint spec:
//! - URL: `${base}/api/tags`
//! - Auth: none (local service)
//! - Response: `{models: [{name, ...}]}` - the field is `name`, the outer key is `models`
//!
//! Match rule: URL contains port `11434` or the `ollama` keyword
//!
//! Reference: LobeHub `packages/model-runtime/src/providers/ollama/index.ts:80-95`

use reqwest::RequestBuilder;

use super::ProviderPlugin;
use crate::core::models::LiveModel;

pub struct OllamaProvider;

impl ProviderPlugin for OllamaProvider {
    fn identifier(&self) -> &'static str {
        "ollama"
    }

    fn matches(&self, api_url: &str) -> bool {
        // Only claims unambiguous Ollama signatures: default port 11434 or "ollama" in the host/id.
        // Claiming any localhost/127.0.0.1 would misclassify local OpenAI-compatible
        // endpoints (LM Studio, llama.cpp server, text-generation-webui) as Ollama
        // and send /api/tags to the wrong endpoint (local engines use the engine_type direct path).
        let u = api_url.to_lowercase();
        u.contains("11434") || u.contains("ollama")
    }

    fn models_url(&self, api_url: &str) -> String {
        format!("{}/api/tags", api_url.trim_end_matches('/'))
    }

    fn apply_auth(&self, req: RequestBuilder, _api_key: &str) -> RequestBuilder {
        // Ollama has no auth
        req
    }

    /// 阶段4 第5项（聊天路径统一出口）: no auth on the /v1 OpenAI-compat chat
    /// endpoint either (the historic chat path sent a meaningless empty Bearer).
    fn apply_chat_auth(&self, req: RequestBuilder, _api_key: &str) -> RequestBuilder {
        req
    }

    fn chat_completions_url(&self, api_url: &str) -> String {
        let base = api_url.trim_end_matches('/');
        // Ollama's OpenAI-compatible endpoint lives under the /v1 path
        if base.ends_with("/v1") {
            format!("{}/chat/completions", base)
        } else {
            format!("{}/v1/chat/completions", base)
        }
    }

    fn parse_models(&self, body: &serde_json::Value) -> Vec<LiveModel> {
        let arr = body
            .get("models")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        arr.iter()
            .filter_map(|m| {
                let id = m.get("name").and_then(|x| x.as_str())?.to_string();
                Some(LiveModel {
                    id,
                    owned_by: "ollama".to_string(),
                    display_name: None,
                    released_at: None,
                })
            })
            .collect()
    }
}
