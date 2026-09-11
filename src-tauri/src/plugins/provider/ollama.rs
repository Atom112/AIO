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

use reqwest::RequestBuilder;

use super::ProviderPlugin;
use crate::core::models::LiveModel;

pub struct OllamaProvider;

impl ProviderPlugin for OllamaProvider {
    fn identifier(&self) -> &'static str {
        "ollama"
    }

    fn matches(&self, api_url: &str) -> bool {
        // 仅认领明确的 Ollama 特征：默认端口 11434 或 host/id 中出现 "ollama"。
        // 不再吞掉任意 localhost/127.0.0.1 —— 那会把 LM Studio、llama.cpp server、
        // text-generation-webui 等本地 OpenAI 兼容端点误判为 Ollama，
        // 导致 /api/tags 请求打错端点（本地引擎另有 engine_type 直通路径）。
        let u = api_url.to_lowercase();
        u.contains("11434") || u.contains("ollama")
    }

    fn models_url(&self, api_url: &str) -> String {
        format!("{}/api/tags", api_url.trim_end_matches('/'))
    }

    fn apply_auth(&self, req: RequestBuilder, _api_key: &str) -> RequestBuilder {
        // Ollama 无认证
        req
    }

    fn chat_completions_url(&self, api_url: &str) -> String {
        let base = api_url.trim_end_matches('/');
        // Ollama 的 OpenAI 兼容端点在 /v1 路径下
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
