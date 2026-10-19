//! OpenCode Go provider plugin (opencode.ai hosted inference).
//!
//! Why a dedicated plugin: OpenCode Go's chat-completions surface requires the
//! `x-opencode-session` header — a stable per-conversation session ID the
//! gateway uses for routing and prompt-cache locality. Requests without it are
//! rejected with `400 MissingSessionID` ("cannot be routed efficiently").
//! Model listing needs no session, which is why connection tests and model
//! fetches succeed while actual chats fail — the exact field report this
//! plugin fixes.
//!
//! Everything else (endpoint URL building, Bearer auth, models parsing) is
//! byte-identical to the OpenAI-compatible fallback and delegated to it — the
//! ONLY behavioral delta is the session header on chat requests.
//!
//! Scope: claims `opencode.ai` hosts whose path is NOT Anthropic/Google-protocol
//! shaped. The Anthropic-protocol Go surface (`/inference/go/anthropic/v1`,
//! serving Claude Haiku / MiniMax M3 / Qwen3.8) needs the Anthropic
//! request+stream protocol, which only the native `api.anthropic.com` plugin
//! implements today; claiming it here without that protocol would send
//! OpenAI-shaped requests to an Anthropic endpoint. Those URLs stay on the
//! fallback (unchanged pre-existing behavior) — tracked in the follow-up issue.

use reqwest::RequestBuilder;
use std::sync::LazyLock;

use super::ProviderPlugin;
use crate::core::models::LiveModel;

/// Conversation-less auxiliary calls (title generation / BTW / memory pipeline)
/// still must identify a session to OpenCode Go. A per-process UUID: stable for
/// the app's lifetime (the documented requirement), and it clusters background
/// traffic away from per-topic sessions so conversation cache routing stays clean.
static FALLBACK_SESSION: LazyLock<String> = LazyLock::new(|| uuid::Uuid::new_v4().to_string());

pub struct OpenCodeProvider;

/// Path-level protocol guard: `/anthropic/` and `/google/` Go surfaces speak
/// native vendor protocols this plugin must not claim (see module doc).
fn is_native_protocol_path(path: &str) -> bool {
    path.contains("/anthropic/") || path.contains("/google/")
}

impl ProviderPlugin for OpenCodeProvider {
    fn identifier(&self) -> &'static str {
        "opencode"
    }

    fn matches(&self, api_url: &str) -> bool {
        if !super::url_host_matches(api_url, "opencode.ai") {
            return false;
        }
        // Host matched; only native-protocol paths are left unclaimed. Mirrors
        // url_host_matches' parse fallback for scheme-less input.
        let parsed = if api_url.contains("://") {
            url::Url::parse(api_url)
        } else {
            url::Url::parse(&format!("https://{api_url}"))
        };
        match parsed {
            Ok(u) => !is_native_protocol_path(u.path()),
            // Unparseable with a matching host is impossible (host check parsed
            // it first); be conservative anyway and do not claim.
            Err(_) => false,
        }
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

    /// OpenCode Go contract: every chat-completions request carries
    /// `x-opencode-session`. `session_id` is the caller's conversation identity
    /// (AIO topic id — stable across app restarts, which preserves the
    /// gateway-side session across sessions too); `None` for conversation-less
    /// auxiliary calls falls back to the per-process session.
    fn apply_chat_session(&self, req: RequestBuilder, session_id: Option<&str>) -> RequestBuilder {
        let session = session_id.unwrap_or(&FALLBACK_SESSION);
        req.header("x-opencode-session", session)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The documented Go OpenAI-compatible surface (and its subdomain form) is claimed.
    #[test]
    fn claims_opencode_go_openai_surface() {
        let p = OpenCodeProvider;
        assert!(p.matches("https://opencode.ai/inference/go/openai/v1"));
        assert!(p.matches("https://opencode.ai/inference/go/openai/v1/"));
        assert!(p.matches("https://opencode.ai/inference/go/v1"));
        assert!(p.matches("https://go.opencode.ai/v1"));
        assert!(p.matches("opencode.ai/inference/go/openai/v1"));
    }

    /// Native-protocol Go paths are NOT claimed (they need the Anthropic/Google
    /// protocol adapters this plugin does not provide) and look-alike hosts
    /// never claim any dedicated plugin (PROV-5 family).
    #[test]
    fn does_not_claim_native_protocol_paths_or_lookalikes() {
        let p = OpenCodeProvider;
        assert!(!p.matches("https://opencode.ai/inference/go/anthropic/v1"));
        assert!(!p.matches("https://opencode.ai/inference/go/google/v1beta/models"));
        assert!(!p.matches("https://opencode.ai.evil.io/v1"));
        assert!(!p.matches("https://evil.io/proxy/opencode.ai"));
    }

    /// The header must land with the caller's session and, when the caller has
    /// no conversation context, with a value that stays stable across calls
    /// (per-process LazyLock — the documented "stable session ID" requirement).
    #[test]
    fn session_header_present_and_stable() {
        let client = reqwest::Client::new();
        let p = OpenCodeProvider;

        let with_topic = p
            .apply_chat_session(client.post("https://opencode.ai/x"), Some("topic-42"))
            .build()
            .expect("build");
        assert_eq!(
            with_topic
                .headers()
                .get("x-opencode-session")
                .and_then(|v| v.to_str().ok()),
            Some("topic-42")
        );

        let fb1 = p
            .apply_chat_session(client.post("https://opencode.ai/x"), None)
            .build()
            .expect("build");
        let fb2 = p
            .apply_chat_session(client.post("https://opencode.ai/x"), None)
            .build()
            .expect("build");
        let v1 = fb1
            .headers()
            .get("x-opencode-session")
            .and_then(|v| v.to_str().ok());
        let v2 = fb2
            .headers()
            .get("x-opencode-session")
            .and_then(|v| v.to_str().ok());
        assert!(v1.is_some());
        assert_eq!(v1, v2, "fallback session must be stable across calls");
    }

    /// URL building and auth stay delegated (byte-identical to the fallback):
    /// a Go base URL produces the documented chat-completions endpoint with
    /// Bearer auth, and the models endpoint keeps the shape model fetching
    /// already uses today.
    #[test]
    fn url_building_delegates_to_openai_compat() {
        let p = OpenCodeProvider;
        assert_eq!(
            p.chat_completions_url("https://opencode.ai/inference/go/openai/v1"),
            "https://opencode.ai/inference/go/openai/v1/chat/completions"
        );
        assert_eq!(
            p.models_url("https://opencode.ai/inference/go/openai/v1"),
            "https://opencode.ai/inference/go/openai/v1/models"
        );

        let client = reqwest::Client::new();
        let req = p
            .apply_auth(client.post("https://opencode.ai/x"), "sk-test")
            .build()
            .expect("build");
        assert_eq!(
            req.headers()
                .get("authorization")
                .and_then(|v| v.to_str().ok()),
            Some("Bearer sk-test")
        );
    }

    /// Dispatch through the manager: Go OpenAI-compatible URLs reach this plugin
    /// (registered before the fallback), Anthropic-shaped Go URLs do not.
    #[test]
    fn manager_dispatches_go_urls_to_opencode_plugin() {
        let mgr = super::super::ProviderManager::new();
        assert_eq!(
            mgr.for_url("https://opencode.ai/inference/go/openai/v1")
                .identifier(),
            "opencode"
        );
        assert_eq!(
            mgr.for_url("https://opencode.ai/inference/go/anthropic/v1")
                .identifier(),
            "openai_compat"
        );
    }
}
