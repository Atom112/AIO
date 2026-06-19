//! Ollama provider plugin (本地推理)
//!
//! 端点规范:
//! - URL: `${base}/api/tags`
//! - Auth: 无 (本地服务)
//! - Response: `{models: [{name, ...}]}` —— 字段名是 `name`，外层是 `models`
//!
//! 匹配规则: URL 含 `11434` 端口 / `localhost` / `127.0.0.1` / `ollama` 关键字
//!
//! 参考 LobeHub `packages/model-runtime/src/providers/ollama/index.ts:80-95`

use reqwest::{Client, RequestBuilder};
use std::time::Duration;

use super::ProviderPlugin;
use crate::core::models::LiveModel;

pub struct OllamaProvider;

impl ProviderPlugin for OllamaProvider {
    fn identifier(&self) -> &'static str {
        "ollama"
    }

    fn name(&self) -> &'static str {
        "Ollama"
    }

    fn matches(&self, api_url: &str) -> bool {
        let u = api_url.to_lowercase();
        u.contains("11434")
            || u.contains("localhost")
            || u.contains("127.0.0.1")
            || u.contains("ollama")
    }

    fn build_client(
        &self,
        proxy_url: Option<&str>,
        timeout_secs: u64,
    ) -> Result<Client, String> {
        let mut b = Client::builder()
            .user_agent("AIO-Desktop/0.4 (ollama-provider)")
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
        format!("{}/api/tags", api_url.trim_end_matches('/'))
    }

    fn apply_auth(&self, req: RequestBuilder, _api_key: &str) -> RequestBuilder {
        // Ollama 无认证
        req
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
