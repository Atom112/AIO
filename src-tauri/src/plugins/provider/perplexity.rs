//! Perplexity provider plugin (api.perplexity.ai).
//!
//! Why a dedicated plugin: Perplexity's current API surface documents chat
//! completions ONLY at `/router/v1/chat/completions` (the Orchestrator API;
//! the pre-reorg Sonar path `/chat/completions` appears nowhere in current
//! docs), while the models listing `GET /v1/models` remains documented and
//! Bearer-authed — exactly the "model fetch succeeds, chat fails" failure
//! mode. The plugin remaps bare/legacy bases to the documented Orchestrator
//! path; bases already carrying `/router` compose with the default rule.
//!
//! Everything else (Bearer auth, models URL building, models parsing)
//! delegates to the OpenAI-compatible fallback — byte-identical behavior.

use reqwest::RequestBuilder;

use super::ProviderPlugin;
use crate::core::models::LiveModel;

pub struct PerplexityProvider;

impl ProviderPlugin for PerplexityProvider {
    fn identifier(&self) -> &'static str {
        "perplexity"
    }

    fn matches(&self, api_url: &str) -> bool {
        super::url_host_matches(api_url, "api.perplexity.ai")
    }

    fn models_url(&self, api_url: &str) -> String {
        super::openai_compat::OpenAICompatProvider.models_url(api_url)
    }

    /// Chat endpoint remap (see module doc):
    /// - bases already carrying `/router` (docs-shaped) compose via the default
    ///   rule: `{base}/chat/completions`;
    /// - bare hosts, legacy `/v1` bases, and full legacy chat paths remap to
    ///   the documented Orchestrator endpoint `/router/v1/chat/completions`.
    fn chat_completions_url(&self, api_url: &str) -> String {
        let trimmed = api_url.trim_end_matches('/');
        // A pasted full legacy chat path loses its suffix first (same handling
        // as the openai_compat models_url fix for full-path bases).
        let trimmed = trimmed
            .strip_suffix("/chat/completions")
            .unwrap_or(trimmed)
            .trim_end_matches('/');
        if trimmed.contains("/router") {
            return super::openai_compat::OpenAICompatProvider.chat_completions_url(api_url);
        }
        let base = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
        format!("{}/router/v1/chat/completions", base)
    }

    fn apply_auth(&self, req: RequestBuilder, api_key: &str) -> RequestBuilder {
        super::openai_compat::OpenAICompatProvider.apply_auth(req, api_key)
    }

    fn parse_models(&self, body: &serde_json::Value) -> Vec<LiveModel> {
        super::openai_compat::OpenAICompatProvider.parse_models(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every input shape users realistically paste lands on the documented
    /// Orchestrator chat endpoint.
    #[test]
    fn chat_url_remaps_to_documented_orchestrator_path() {
        let p = PerplexityProvider;
        // Bare host (the pre-reorg default everyone has memorized)
        assert_eq!(
            p.chat_completions_url("https://api.perplexity.ai"),
            "https://api.perplexity.ai/router/v1/chat/completions"
        );
        // Legacy /v1 base (the catalog's perplexity-agent entry shape)
        assert_eq!(
            p.chat_completions_url("https://api.perplexity.ai/v1"),
            "https://api.perplexity.ai/router/v1/chat/completions"
        );
        // Full legacy chat path
        assert_eq!(
            p.chat_completions_url("https://api.perplexity.ai/chat/completions"),
            "https://api.perplexity.ai/router/v1/chat/completions"
        );
        // Docs-shaped base (already carries /router): default composition, no remap
        assert_eq!(
            p.chat_completions_url("https://api.perplexity.ai/router/v1"),
            "https://api.perplexity.ai/router/v1/chat/completions"
        );
    }

    /// Models URL stays the documented Agent-API list endpoint for the common
    /// bases (delegated rule: bare host -> /v1/models, /v1 base -> /models).
    #[test]
    fn models_url_delegates_to_openai_compat() {
        let p = PerplexityProvider;
        assert_eq!(
            p.models_url("https://api.perplexity.ai"),
            "https://api.perplexity.ai/v1/models"
        );
        assert_eq!(
            p.models_url("https://api.perplexity.ai/v1"),
            "https://api.perplexity.ai/v1/models"
        );
    }

    /// Host claim + dispatch: the genuine host reaches this plugin; look-alike
    /// hosts never do (PROV-5 family).
    #[test]
    fn manager_dispatches_perplexity_host() {
        let p = PerplexityProvider;
        assert!(p.matches("https://api.perplexity.ai"));
        assert!(p.matches("https://api.perplexity.ai/router/v1"));
        assert!(!p.matches("https://api.perplexity.ai.evil.io"));
        assert!(!p.matches("https://evil.io/proxy/api.perplexity.ai"));

        let mgr = super::super::ProviderManager::new();
        assert_eq!(
            mgr.for_url("https://api.perplexity.ai").identifier(),
            "perplexity"
        );
    }
}
