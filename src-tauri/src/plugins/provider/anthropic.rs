//! Anthropic provider plugin
//!
//! 端点规范:
//! - URL: `${base}/v1/models` (base = `https://api.anthropic.com`, 不带版本号)
//! - Auth: `x-api-key: <key>` + `anthropic-version: 2023-06-01` (不能用 Authorization: Bearer)
//! - Response: `{data: [{id, display_name, created_at}]}` —— 用 `id` 字段，和 OpenAI 兼容
//!
//! 参考 LobeHub `packages/model-runtime/src/core/anthropicCompatibleFactory/index.ts:286-310`

use reqwest::{Client, RequestBuilder};
use std::time::Duration;

use super::ProviderPlugin;
use crate::core::models::LiveModel;

pub struct AnthropicProvider;

impl ProviderPlugin for AnthropicProvider {
    fn identifier(&self) -> &'static str {
        "anthropic"
    }

    fn name(&self) -> &'static str {
        "Anthropic"
    }

    fn matches(&self, api_url: &str) -> bool {
        let u = api_url.to_lowercase();
        u.contains("api.anthropic.com")
    }

    fn build_client(
        &self,
        proxy_url: Option<&str>,
        timeout_secs: u64,
    ) -> Result<Client, String> {
        let mut b = Client::builder()
            .user_agent("AIO-Desktop/0.4 (anthropic-provider)")
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
        // 用户的 api_url 形如 `https://api.anthropic.com` (无版本号)
        // 或 `https://api.anthropic.com/v1` (带版本号)
        let trimmed = api_url.trim_end_matches('/');
        if trimmed.ends_with("/v1") {
            format!("{}/models", trimmed)
        } else {
            format!("{}/v1/models", trimmed)
        }
    }

    fn apply_auth(&self, req: RequestBuilder, api_key: &str) -> RequestBuilder {
        // Anthropic 强制要求 x-api-key + anthropic-version 两个头
        if api_key.is_empty() {
            req.header("anthropic-version", "2023-06-01")
        } else {
            req.header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
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
                    .get("display_name")
                    .and_then(|x| x.as_str())
                    .unwrap_or("Anthropic")
                    .to_string();
                Some(LiveModel { id, owned_by: owned })
            })
            .collect()
    }

    fn default_models(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("claude-sonnet-4-5", "Anthropic"),
            ("claude-opus-4-1", "Anthropic"),
            ("claude-haiku-4-5", "Anthropic"),
        ]
    }
}
