//! DeepSeek provider plugin (api.deepseek.com).
//!
//! Why a dedicated plugin: DeepSeek V4-series reasoning models run a **documented
//! tool-call contract** that plain OpenAI-compat providers violate (harness
//! feedback round 3, from a real-world V4.1 Flash report — "thinks for tens of
//! minutes, re-reads dozens of files, never acts"):
//!
//! 1. **`reasoning_content` round-trip**: when a request carries `tools`, every
//!    assistant message in the conversation must carry its `reasoning_content`
//!    back to the API ("让模型继续之前的思考") — it is the model's cross-round
//!    WORKING MEMORY. Dropping it (the historic behavior) made each round start
//!    thinking from scratch and re-read files to re-establish understanding: the
//!    re-read loop behind the report. OpenAI's strict message schema rejects the
//!    unknown field, so the round-trip must stay provider-scoped — hence this
//!    plugin's [`ProviderPlugin::roundtrip_reasoning_content`] override.
//! 2. **Thinking is ON by default at `high` effort** (per the thinking-mode
//!    docs) and can only be disabled via `{"thinking": {"type": "disabled"}}` —
//!    a DeepSeek-own parameter AIO historically never sent, so the reasoning
//!    slider's "off" was a no-op for DeepSeek. This plugin's
//!    [`ProviderPlugin::adjust_chat_body`] sends it when the caller passes no
//!    reasoning effort, and maps the slider's levels onto DeepSeek's actual
//!    effort ladder (medium collapses to high upstream — we send low instead,
//!    since DeepSeek's real ladder is low/high/max).
//!
//! Everything else (URL building, Bearer auth, models parsing) delegates to the
//! OpenAI-compatible fallback — byte-identical behavior.
//!
//! Reference: https://api-docs.deepseek.com/zh-cn/guides/thinking_mode/

use reqwest::RequestBuilder;
use serde_json::{json, Map, Value};

use super::ProviderPlugin;
use crate::core::models::LiveModel;

pub struct DeepSeekProvider;

impl ProviderPlugin for DeepSeekProvider {
    fn identifier(&self) -> &'static str {
        "deepseek"
    }

    fn matches(&self, api_url: &str) -> bool {
        super::url_host_matches(api_url, "api.deepseek.com")
    }

    fn models_url(&self, api_url: &str) -> String {
        super::openai_compat::OpenAICompatProvider.models_url(api_url)
    }

    fn apply_auth(&self, req: RequestBuilder, api_key: &str) -> RequestBuilder {
        super::openai_compat::OpenAICompatProvider.apply_auth(req, api_key)
    }

    fn parse_models(&self, body: &serde_json::Value) -> Vec<LiveModel> {
        super::openai_compat::OpenAICompatProvider.parse_models(body)
    }

    /// DeepSeek tool-call contract: with `tools` in play, the model's
    /// `reasoning_content` must ride along on every assistant message in the
    /// conversation. Scoped to the provider's own reasoning model families —
    /// `deepseek-chat` (non-thinking) round-trips an absent/empty field anyway,
    /// which the API ignores; keep the flag tight to the deepseek namespace so
    /// other vendors' strict schemas never see the field.
    fn roundtrip_reasoning_content(&self, _model: &str) -> bool {
        true
    }

    /// Thinking-mode control (see module doc): the caller's reasoning effort
    /// maps onto DeepSeek's ladder, and `None` (slider "off") explicitly DISABLES
    /// thinking — the only way the "off" promise can be kept for this provider.
    /// Model-agnostic on purpose: the `thinking`/`reasoning_effort` semantics are
    /// properties of the API endpoint, not the individual model id; non-reasoning
    /// models (`deepseek-chat`) accept and ignore them.
    fn adjust_chat_body(&self, body: &mut Map<String, Value>, reasoning_effort: Option<&str>) {
        match reasoning_effort {
            // Slider "off": the documented switch. Without this DeepSeek defaults
            // to thinking at HIGH effort and the slider is a no-op.
            None => {
                body.insert("thinking".into(), json!({"type": "disabled"}));
            }
            // DeepSeek's real ladder is low/high/max; "medium" maps to HIGH
            // upstream, so the slider's middle step collapses to low to keep
            // three distinguishable settings.
            Some("low") | Some("medium") => {
                body.insert("reasoning_effort".into(), json!("low"));
            }
            Some(other) => {
                body.insert("reasoning_effort".into(), json!(other));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claims_deepseek_host_only() {
        let p = DeepSeekProvider;
        assert!(p.matches("https://api.deepseek.com"));
        assert!(p.matches("https://api.deepseek.com/v1"));
        // Look-alikes fall through to the fallback (PROV-5 family)
        assert!(!p.matches("https://api.deepseek.com.evil.io"));
        assert!(!p.matches("https://evil.io/proxy/api.deepseek.com"));
        assert!(!p.matches("https://api.openai.com"));

        let mgr = super::super::ProviderManager::new();
        assert_eq!(
            mgr.for_url("https://api.deepseek.com").identifier(),
            "deepseek"
        );
    }

    /// off → thinking:disabled; low/medium → reasoning_effort low; high → high.
    /// (The ladder: DeepSeek accepts low/high/max; upstream maps medium→high,
    /// so the slider's medium must be sent as low to stay distinguishable.)
    #[test]
    fn adjust_chat_body_maps_reasoning_ladder() {
        let p = DeepSeekProvider;
        let mut b: Map<String, Value> = Map::new();
        p.adjust_chat_body(&mut b, None);
        assert_eq!(b.get("thinking"), Some(&json!({"type": "disabled"})));
        assert!(b.get("reasoning_effort").is_none());

        let mut b: Map<String, Value> = Map::new();
        p.adjust_chat_body(&mut b, Some("medium"));
        assert_eq!(b.get("reasoning_effort"), Some(&json!("low")));

        let mut b: Map<String, Value> = Map::new();
        p.adjust_chat_body(&mut b, Some("low"));
        assert_eq!(b.get("reasoning_effort"), Some(&json!("low")));

        let mut b: Map<String, Value> = Map::new();
        p.adjust_chat_body(&mut b, Some("high"));
        assert_eq!(b.get("reasoning_effort"), Some(&json!("high")));
        assert!(b.get("thinking").is_none());
    }

    /// URL building and auth delegate unchanged to the fallback.
    #[test]
    fn url_building_delegates_to_openai_compat() {
        let p = DeepSeekProvider;
        assert_eq!(
            p.models_url("https://api.deepseek.com"),
            "https://api.deepseek.com/v1/models"
        );
        assert_eq!(
            p.chat_completions_url("https://api.deepseek.com"),
            "https://api.deepseek.com/chat/completions"
        );
    }
}
