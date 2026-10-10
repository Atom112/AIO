//! Google Gemini provider plugin
//!
//! Endpoint spec:
//! - URL: `${base}/v1beta/models` (base carries no version)
//! - Auth: `x-goog-api-key: <key>` (must not use Authorization: Bearer)
//! - Response: `{models: [{name: "models/gemini-...", displayName, ...}]}`
//!   the field is `name` (strip the `models/` prefix), not `id`; the outer key is `models`, not `data`
//!
//! Reference: LobeHub `packages/model-runtime/src/providers/google/index.ts:351-365`

use reqwest::RequestBuilder;

use super::ProviderPlugin;
use crate::core::models::LiveModel;

pub struct GoogleProvider;

impl ProviderPlugin for GoogleProvider {
    fn identifier(&self) -> &'static str {
        "google"
    }

    fn matches(&self, api_url: &str) -> bool {
        let u = api_url.to_lowercase();
        u.contains("generativelanguage.googleapis.com")
    }

    fn models_url(&self, api_url: &str) -> String {
        // The user's api_url looks like `https://generativelanguage.googleapis.com/v1beta`
        // strip the trailing version before appending `/v1beta/models`
        let trimmed = api_url.trim_end_matches('/');
        if trimmed.ends_with("/v1beta") || trimmed.ends_with("/v1") {
            // Already versioned: strip the version, then append the standard path
            let base = trimmed
                .trim_end_matches("/v1beta")
                .trim_end_matches("/v1")
                .trim_end_matches('/');
            format!("{}/v1beta/models", base)
        } else {
            // Bare host: append directly
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

    /// PROV-3: Gemini 的 OpenAI 兼容聊天端点是 `${base}/v1beta/openai/chat/completions`。
    /// trait 默认实现拼 `${base}/chat/completions`（Google 无此路径 → 默认配置聊天
    /// 必 404）；只覆写 models_url 时两者互斥（用户填 `/v1beta/openai` 则拉模型 404）。
    fn chat_completions_url(&self, api_url: &str) -> String {
        // 用户 api_url 形如 `https://generativelanguage.googleapis.com/v1beta`
        // （或已带 /v1beta/openai、/chat/completions 后缀的各种历史写法）——
        // 先剥掉尾部路径段回到 bare host，再拼标准端点。
        let mut base = api_url.trim_end_matches('/').to_string();
        for suffix in [
            "/chat/completions",
            "/openai/chat/completions",
            "/openai",
            "/v1beta",
            "/v1",
        ] {
            if base.ends_with(suffix) {
                base = base
                    .strip_suffix(suffix)
                    .unwrap_or(&base)
                    .trim_end_matches('/')
                    .to_string();
            }
        }
        format!("{}/v1beta/openai/chat/completions", base)
    }

    fn parse_models(&self, body: &serde_json::Value) -> Vec<LiveModel> {
        let arr = body
            .get("models")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        arr.iter()
            .filter_map(|m| {
                // Gemini's returned name looks like "models/gemini-2.5-pro"
                let raw_name = m.get("name").and_then(|x| x.as_str())?.to_string();
                let id = raw_name
                    .strip_prefix("models/")
                    .unwrap_or(&raw_name)
                    .to_string();
                // displayName is the human-readable name (e.g. "Gemini 2.5 Pro"); map it to display_name
                let display_name = m
                    .get("displayName")
                    .and_then(|x| x.as_str())
                    .map(String::from);
                // Gemini responses have no standard release-date field; keep None
                Some(LiveModel {
                    id,
                    owned_by: "Google".to_string(),
                    display_name,
                    released_at: None,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PROV-3 回归：默认配置（/v1beta）下聊天端点必须是 OpenAI 兼容路径，
    /// 而不是 trait 默认拼的 `/v1beta/chat/completions`（Google 不存在该路径）。
    #[test]
    fn chat_completions_url_uses_openai_compat_path() {
        let p = GoogleProvider;
        assert_eq!(
            p.chat_completions_url("https://generativelanguage.googleapis.com/v1beta"),
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
        // 已带 openai 段 / 完整端点 / bare host 的历史写法都归一到同一端点
        assert_eq!(
            p.chat_completions_url("https://generativelanguage.googleapis.com/v1beta/openai"),
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
        assert_eq!(
            p.chat_completions_url(
                "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
            ),
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
        assert_eq!(
            p.chat_completions_url("https://generativelanguage.googleapis.com"),
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
        // models_url 与 chat 端点不再互斥：同一个 /v1beta 前缀两个端点都能用
        assert!(p
            .models_url("https://generativelanguage.googleapis.com/v1beta")
            .ends_with("/v1beta/models"));
    }
}
