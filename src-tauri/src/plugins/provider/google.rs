//! Google Gemini provider plugin
//!
//! 端点规范:
//! - URL: `${base}/v1beta/models` (base 不带版本)
//! - Auth: `x-goog-api-key: <key>` (不能用 Authorization: Bearer)
//! - Response: `{models: [{name: "models/gemini-...", displayName, ...}]}`
//!   字段名是 `name` (需要剥掉 `models/` 前缀)，不是 `id`；外层是 `models`，不是 `data`
//!
//! 参考 LobeHub `packages/model-runtime/src/providers/google/index.ts:351-365`

use reqwest::{Client, RequestBuilder};
use std::time::Duration;

use super::ProviderPlugin;
use crate::core::models::LiveModel;

pub struct GoogleProvider;

impl ProviderPlugin for GoogleProvider {
    fn identifier(&self) -> &'static str {
        "google"
    }

    fn name(&self) -> &'static str {
        "Google Gemini"
    }

    fn matches(&self, api_url: &str) -> bool {
        let u = api_url.to_lowercase();
        u.contains("generativelanguage.googleapis.com")
    }

    fn build_client(
        &self,
        proxy_url: Option<&str>,
        timeout_secs: u64,
    ) -> Result<Client, String> {
        let mut b = Client::builder()
            .user_agent("AIO-Desktop/0.4 (google-provider)")
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
        // 用户的 api_url 形如 `https://generativelanguage.googleapis.com/v1beta`
        // 需要去掉尾部版本号后再拼 `/v1beta/models`
        let trimmed = api_url.trim_end_matches('/');
        if trimmed.ends_with("/v1beta") || trimmed.ends_with("/v1") {
            // 已经是带版本的，去掉版本号再拼标准路径
            let base = trimmed
                .trim_end_matches("/v1beta")
                .trim_end_matches("/v1")
                .trim_end_matches('/');
            format!("{}/v1beta/models", base)
        } else {
            // 裸 host，直接拼
            format!("{}/v1beta/models", trimmed)
        }
    }

    fn apply_auth(&self, req: RequestBuilder, api_key: &str) -> RequestBuilder {
        if api_key.is_empty() {
            req
        } else {
            req.header("x-goog-api-key", api_key)
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
                // Gemini 返回的 name 形如 "models/gemini-2.5-pro"
                let raw_name = m.get("name").and_then(|x| x.as_str())?.to_string();
                let id = raw_name
                    .strip_prefix("models/")
                    .unwrap_or(&raw_name)
                    .to_string();
                let display_name = m
                    .get("displayName")
                    .and_then(|x| x.as_str())
                    .map(String::from);
                // Gemini 不返回发布信息
                Some(LiveModel {
                    id,
                    owned_by: "Google".to_string(),
                    display_name,
                    released_at: None,
                })
            })
            .collect()
    }

    fn default_models(&self) -> Vec<(&'static str, &'static str)> {
        vec![
            ("gemini-2.5-pro", "Google"),
            ("gemini-2.5-flash", "Google"),
            ("gemini-2.0-flash-exp", "Google"),
        ]
    }
}
