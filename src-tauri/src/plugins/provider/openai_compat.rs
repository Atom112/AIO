//! OpenAI 兼容 provider plugin (兜底)
//!
//! 覆盖: OpenAI / DeepSeek / Groq / Mistral / xAI / Cohere / OpenRouter / 自定义 OpenAI-兼容
//!
//! 端点规范:
//! - URL: 智能拼接 `/v1/models` 或 `/models`
//! - Auth: `Authorization: Bearer <key>`
//! - Response: `{data: [{id, owned_by}]}`
//!
//! 必须是最后一个注册 —— 兜底插件。

use reqwest::{Client, RequestBuilder};
use std::time::Duration;

use super::ProviderPlugin;
use crate::core::models::LiveModel;

pub struct OpenAICompatProvider;

impl ProviderPlugin for OpenAICompatProvider {
    fn identifier(&self) -> &'static str {
        "openai_compat"
    }

    fn name(&self) -> &'static str {
        "OpenAI Compatible"
    }

    fn matches(&self, _api_url: &str) -> bool {
        // 永远不主动匹配，仅作为 manager 兜底
        false
    }

    fn build_client(
        &self,
        proxy_url: Option<&str>,
        timeout_secs: u64,
    ) -> Result<Client, String> {
        let mut b = Client::builder()
            .user_agent("AIO-Desktop/0.4 (openai-compat-provider)")
            .timeout(Duration::from_secs(timeout_secs));
        if let Some(p) = proxy_url {
            if !p.trim().is_empty() {
                let proxy = reqwest::Proxy::all(p).map_err(|e| format!("代理 URL 非法: {}", e))?;
                b = b.proxy(proxy);
            }
        }
        b.build().map_err(|e| format!("构造 HTTP 客户端失败: {}", e))
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
                Some(LiveModel { id, owned_by: owned })
            })
            .collect()
    }

    fn default_models(&self) -> Vec<(&'static str, &'static str)> {
        // 兜底: 列出最常见的 OpenAI 兼容厂商预设
        vec![
            ("gpt-4o", "OpenAI"),
            ("gpt-4o-mini", "OpenAI"),
            ("o3", "OpenAI"),
            ("o4-mini", "OpenAI"),
            ("deepseek-v3", "DeepSeek"),
            ("llama-3.3-70b-versatile", "Groq"),
            ("mistral-large-latest", "Mistral"),
            ("grok-4", "xAI"),
            ("command-r-plus", "Cohere"),
        ]
    }
}
