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
