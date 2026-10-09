//! # Provider plugin system
//!
//! LobeHub-style architecture: each provider (OpenAI / Anthropic / Google Gemini / Ollama, etc.)
//! implements the [`ProviderPlugin`] trait, describing its specifics:
//! - endpoint URL construction rules
//! - HTTP auth headers (Bearer / x-goog-api-key / x-api-key / no auth)
//! - response JSON field mapping (`{data:[{id}]}` / `{models:[{name}]}` / `{data:[{id,display_name}]}`)
//! - hardcoded default model list used when fetching fails
//!
//! [`ProviderManager`] dispatches by host to the first matching plugin; with no match it falls back to [`openai_compat`].
//!
//! Mirrors the local-inference engine plugin shape of [`crate::plugins::engine`] (synchronous trait;
//! internal I/O is done by the caller within async commands), avoiding an `async_trait` dependency.

pub mod anthropic;
pub mod google;
pub mod ollama;
pub mod openai_compat;

use crate::core::models::LiveModel;
use crate::ierr;

/// Builds a reqwest client with timeout and proxy (shared implementation for all four plugins).
/// The user-agent carries the crate version, avoiding a hardcoded version drifting out of date across releases.
pub fn build_http_client(
    proxy_url: Option<&str>,
    timeout_secs: u64,
    provider_tag: &str,
) -> Result<reqwest::Client, String> {
    let mut b = reqwest::Client::builder()
        .user_agent(format!(
            "AIO-Desktop/{} ({}-provider)",
            env!("CARGO_PKG_VERSION"),
            provider_tag
        ))
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(timeout_secs));
    if let Some(p) = proxy_url {
        if !p.trim().is_empty() {
            let proxy = reqwest::Proxy::all(p)
                .map_err(|e| ierr!("代理 URL 非法: {}", "Invalid proxy URL: {}", e))?;
            b = b.proxy(proxy);
        }
    }
    b.build().map_err(|e| {
        ierr!(
            "构造 HTTP 客户端失败: {}",
            "Failed to build HTTP client: {}",
            e
        )
    })
}

/// Provider plugin trait - per-vendor API adapter
pub trait ProviderPlugin: Send + Sync {
    /// Machine identifier, e.g. "google" / "anthropic" / "openai_compat" / "ollama"
    fn identifier(&self) -> &'static str;

    /// Decides whether this plugin applies to the given api_url by matching its host
    fn matches(&self, api_url: &str) -> bool;

    /// Builds a reqwest client with timeout and proxy (default implementation = build_http_client with identifier as tag)
    fn build_client(
        &self,
        proxy_url: Option<&str>,
        timeout_secs: u64,
    ) -> Result<reqwest::Client, String> {
        build_http_client(proxy_url, timeout_secs, self.identifier())
    }

    /// Computes the list-models endpoint URL
    fn models_url(&self, api_url: &str) -> String;

    /// Attaches auth headers to the RequestBuilder
    fn apply_auth(&self, req: reqwest::RequestBuilder, api_key: &str) -> reqwest::RequestBuilder;

    /// Parses the JSON response into the unified LiveModel list
    fn parse_models(&self, body: &serde_json::Value) -> Vec<LiveModel>;

    /// Computes the chat completions endpoint URL (including provider-specific path prefixes).
    /// Default behavior: strip trailing slashes and any existing /chat/completions suffix, then append /chat/completions.
    /// Providers like Ollama can override this to insert prefixes such as /v1.
    fn chat_completions_url(&self, api_url: &str) -> String {
        let base = api_url
            .trim_end_matches('/')
            .replace("/chat/completions", "");
        format!("{}/chat/completions", base)
    }
}

/// Provider plugin manager: dispatches by host to the first matching plugin
pub struct ProviderManager {
    plugins: Vec<Box<dyn ProviderPlugin>>,
}

impl ProviderManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            plugins: Vec::new(),
        };
        // Registration order matters: dedicated plugins first; openai_compat must stay last as the fallback
        mgr.register(Box::new(google::GoogleProvider));
        mgr.register(Box::new(anthropic::AnthropicProvider));
        mgr.register(Box::new(ollama::OllamaProvider));
        mgr.register(Box::new(openai_compat::OpenAICompatProvider));
        mgr
    }

    pub fn register(&mut self, plugin: Box<dyn ProviderPlugin>) {
        self.plugins.push(plugin);
    }

    /// Finds the first plugin matching the api_url; returns the last one when nothing matches (openai_compat fallback)
    pub fn for_url(&self, api_url: &str) -> &dyn ProviderPlugin {
        self.plugins
            .iter()
            .find(|p| p.matches(api_url))
            .map(|p| p.as_ref())
            .unwrap_or_else(|| {
                self.plugins
                    .last()
                    .expect("at least one provider plugin must be registered")
                    .as_ref()
            })
    }

    /// Owned version of [`for_url`]: rebuilds a `Box<dyn ProviderPlugin>` by identifier.
    /// Use when the plugin must outlive the manager's scope (e.g. the command layer's "test connection / fetch models");
    /// when adding a new plugin, a branch must be registered here (the order test keeps the fallback last).
    pub fn boxed_for_url(&self, api_url: &str) -> Box<dyn ProviderPlugin> {
        let matched = self.for_url(api_url).identifier();
        match matched {
            "google" => Box::new(google::GoogleProvider),
            "anthropic" => Box::new(anthropic::AnthropicProvider),
            "ollama" => Box::new(ollama::OllamaProvider),
            _ => Box::new(openai_compat::OpenAICompatProvider),
        }
    }
}

#[cfg(test)]
mod manager_tests {
    use super::*;

    /// Registration order invariant: openai_compat (the fallback) must always stay last.
    /// for_url's fallback logic assumes the last plugin is openai_compat; changing the order silently misdispatches.
    #[test]
    fn openai_compat_stays_last() {
        let mgr = ProviderManager::new();
        let last = mgr.plugins.last().unwrap();
        assert_eq!(last.identifier(), "openai_compat");
        // A generic URL must dispatch to the fallback
        assert_eq!(
            mgr.for_url("https://api.example.com/v1").identifier(),
            "openai_compat"
        );
    }
}

/// Classifies a reqwest error into a user-friendly message (locale-aware)
pub fn classify_reqwest_error(err: &reqwest::Error, host: &str) -> String {
    if err.is_timeout() {
        ierr!(
            "请求超时 ({}): 请检查网络或代理设置",
            "Request timed out ({}): check your network or proxy settings",
            host
        )
    } else if err.is_connect() {
        ierr!(
            "网络无法访问 {}: 请检查网络或配置代理 (proxyUrl)",
            "Cannot reach {}: check your network or configure a proxy (proxyUrl)",
            host
        )
    } else if err.is_request() {
        ierr!("请求错误 ({}): {}", "Request error ({}): {}", host, err)
    } else {
        ierr!("HTTP 客户端错误: {}", "HTTP client error: {}", err)
    }
}

/// Standard timeout
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
/// Shorter timeout for connection tests
pub const TEST_TIMEOUT_SECS: u64 = 20;

/// Unix seconds timestamp -> YYYY-MM-DD (for the OpenAI `created` field).
/// Single source of truth: implemented in utils::time.
pub fn unix_to_ymd(secs: i64) -> Option<String> {
    crate::utils::time::unix_to_ymd(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unix_to_ymd() {
        // 2025-01-01 00:00:00 UTC = 1735689600
        assert_eq!(unix_to_ymd(1_735_689_600), Some("2025-01-01".to_string()));
        // 2000-01-01 00:00:00 UTC = 946684800
        assert_eq!(unix_to_ymd(946_684_800), Some("2000-01-01".to_string()));
        // 1970-01-01 00:00:00 UTC = 0
        assert_eq!(unix_to_ymd(0), Some("1970-01-01".to_string()));
    }
}
