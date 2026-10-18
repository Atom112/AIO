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
use std::sync::Arc;

/// Builds a reqwest client with timeout and proxy (shared implementation for all four plugins).
/// The user-agent carries the crate version, avoiding a hardcoded version drifting out of date across releases.
pub fn build_http_client(
    proxy_url: Option<&str>,
    timeout_secs: u64,
    provider_tag: &str,
) -> Result<reqwest::Client, String> {
    build_http_client_pinned(proxy_url, timeout_secs, provider_tag, None)
}

/// SEC-7: `build_http_client` with the DNS resolution pinned to a pre-validated
/// address. The connection-test request carries the user's real API key, so a
/// rebinding flip (benign A record at check time, metadata/internal at connect
/// time) must not be able to redirect it; `utils::ssrf` produces the pin pair.
pub fn build_http_client_pinned(
    proxy_url: Option<&str>,
    timeout_secs: u64,
    provider_tag: &str,
    pin: Option<(String, std::net::SocketAddr)>,
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
    if let Some((host, addr)) = pin {
        b = b.resolve(&host, addr);
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

    /// Auth for the CHAT completions path — 阶段4 第5项（聊天路径统一出口）.
    ///
    /// Distinct from [`apply_auth`] only where a provider runs two API surfaces:
    /// Google's model-list fetch hits the native Gemini endpoint (x-goog-api-key)
    /// while chat goes through the OpenAI-compat endpoint whose documented auth is
    /// Bearer; Anthropic sends its native headers on both. Default = the
    /// OpenAI-compat Bearer convention (empty key sends no header — local engines
    /// need none); Anthropic and Ollama override.
    fn apply_chat_auth(
        &self,
        req: reqwest::RequestBuilder,
        api_key: &str,
    ) -> reqwest::RequestBuilder {
        if api_key.is_empty() {
            req
        } else {
            req.bearer_auth(api_key)
        }
    }

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
    plugins: Vec<Arc<dyn ProviderPlugin>>,
}

impl ProviderManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            plugins: Vec::new(),
        };
        // Registration order matters: dedicated plugins first; openai_compat must stay last as the fallback
        mgr.register(Arc::new(google::GoogleProvider));
        mgr.register(Arc::new(anthropic::AnthropicProvider));
        mgr.register(Arc::new(ollama::OllamaProvider));
        mgr.register(Arc::new(openai_compat::OpenAICompatProvider));
        mgr
    }

    pub fn register(&mut self, plugin: Arc<dyn ProviderPlugin>) {
        self.plugins.push(plugin);
    }

    /// Finds the first plugin matching the api_url; returns the last one when nothing matches (openai_compat fallback).
    /// 阶段4 第5项: returns an OWNED Arc clone, so the plugin outlives the manager —
    /// the historic `boxed_for_url` second registry (identifier → rebuild a Box, one
    /// more list to touch per new plugin, and it silently mis-dispatched plugins that
    /// were registered but not re-mapped) is gone: registering in [`ProviderManager::new`]
    /// is now the single list.
    pub fn for_url(&self, api_url: &str) -> Arc<dyn ProviderPlugin> {
        self.plugins
            .iter()
            .find(|p| p.matches(api_url))
            .cloned()
            .unwrap_or_else(|| {
                self.plugins
                    .last()
                    .expect("at least one provider plugin must be registered")
                    .clone()
            })
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

    /// PROV-5 regression: look-alike hosts must NOT claim the native plugins —
    /// the historic whole-string `contains` dispatch sent x-api-key (Anthropic)
    /// and Bearer credentials to foreign hosts like `api.anthropic.com.evil.io`.
    /// They must fall through to the openai_compat fallback (whose auth is
    /// whatever the user configured for that URL, never a native header set).
    #[test]
    fn lookalike_hosts_do_not_claim_native_plugins() {
        let mgr = ProviderManager::new();
        assert_eq!(
            mgr.for_url("https://api.anthropic.com.evil.io/v1")
                .identifier(),
            "openai_compat"
        );
        assert_eq!(
            mgr.for_url("https://generativelanguage.googleapis.com.evil.io/v1beta")
                .identifier(),
            "openai_compat"
        );
        // The genuine hosts still dispatch to their native plugins.
        assert_eq!(
            mgr.for_url("https://api.anthropic.com/v1").identifier(),
            "anthropic"
        );
        assert_eq!(
            mgr.for_url("https://generativelanguage.googleapis.com/v1beta")
                .identifier(),
            "google"
        );
    }

    /// 阶段4 第5项回归：新注册的插件经 for_url 即刻生效，返回的 Arc 超越管理器存活。
    /// 旧 boxed_for_url 的 identifier→重建第二注册表对新插件一无所知——同一个 URL
    /// 会静默重建出 openai_compat 兜底（错误分发），这是双注册表结构的固有缺陷。
    struct TestProvider;
    impl ProviderPlugin for TestProvider {
        fn identifier(&self) -> &'static str {
            "test_provider"
        }
        fn matches(&self, api_url: &str) -> bool {
            api_url.contains("test.example.com")
        }
        fn models_url(&self, api_url: &str) -> String {
            format!("{api_url}/models")
        }
        fn apply_auth(
            &self,
            req: reqwest::RequestBuilder,
            _api_key: &str,
        ) -> reqwest::RequestBuilder {
            req
        }
        fn parse_models(&self, _body: &serde_json::Value) -> Vec<LiveModel> {
            Vec::new()
        }
    }

    #[test]
    fn for_url_dispatches_newly_registered_plugin_without_a_second_registry() {
        let mut mgr = ProviderManager::new();
        mgr.register(Arc::new(TestProvider));
        let plugin = mgr.for_url("https://test.example.com/v1");
        assert_eq!(plugin.identifier(), "test_provider");
        // The Arc clone outlives the manager (the reason boxed_for_url existed)
        drop(mgr);
        assert_eq!(plugin.identifier(), "test_provider");
        assert_eq!(
            plugin.chat_completions_url("https://test.example.com/v1"),
            "https://test.example.com/v1/chat/completions"
        );
    }
}

/// PROV-5: host-suffix match against the URL authority.
/// The historic whole-string `contains` dispatch let look-alike hosts
/// (`api.anthropic.com.evil.io`) claim the native plugin and receive its auth
/// headers (x-api-key / Bearer) — credential leakage to a foreign host. The
/// host must equal `host` or be a subdomain of it; a URL that cannot be parsed
/// claims no dedicated plugin (falls through to openai_compat).
pub fn url_host_matches(api_url: &str, host: &str) -> bool {
    let parsed = if api_url.contains("://") {
        url::Url::parse(api_url)
    } else {
        url::Url::parse(&format!("https://{api_url}"))
    };
    match parsed
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_lowercase()))
    {
        Some(h) => h == host || h.ends_with(&format!(".{host}")),
        None => false,
    }
}

/// Strips the query string and fragment from a URL (PROV-11): Gemini-style auth puts
/// the API key in `?key=…`, and full URLs otherwise end up verbatim in toasts and
/// logs when errors are echoed. Non-URL input is returned unchanged.
pub fn redact_url(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(parsed) => {
            let mut out = format!(
                "{}://{}",
                parsed.scheme(),
                parsed.host_str().unwrap_or("host")
            );
            if let Some(port) = parsed.port() {
                out = format!("{out}:{port}");
            }
            out.push_str(parsed.path());
            out
        }
        Err(_) => url.to_string(),
    }
}

/// Replaces every URL embedded in free-form text (error bodies, reqwest Display
/// strings) with its redacted form (PROV-11) — credentials in query strings never
/// reach the UI or the logs.
pub fn redact_urls_in(text: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r#"https?://[^\s'"<>()\[\]]+"#).expect("固定 URL 正则必然编译成功")
    });
    re.replace_all(text, |caps: &regex::Captures| redact_url(&caps[0]))
        .into_owned()
}

/// Classifies a reqwest error into a user-friendly message (locale-aware).
/// PROV-11: `host` may actually be a full request URL (callers pass api_url), and
/// reqwest's Display carries the complete URL — both are redacted before display.
pub fn classify_reqwest_error(err: &reqwest::Error, host: &str) -> String {
    let host = redact_url(host);
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
        ierr!(
            "请求错误 ({}): {}",
            "Request error ({}): {}",
            host,
            redact_urls_in(&err.to_string())
        )
    } else {
        ierr!(
            "HTTP 客户端错误: {}",
            "HTTP client error: {}",
            redact_urls_in(&err.to_string())
        )
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

    /// PROV-11: URLs echoed in error text are redacted before display — the
    /// query string (Gemini `?key=…`) never reaches toasts or logs.
    #[test]
    fn url_redaction_strips_credentials() {
        assert_eq!(
            redact_url(
                "https://generativelanguage.googleapis.com/v1beta/models?key=AIzaSyVERYSECRET"
            ),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );
        assert_eq!(
            redact_url("http://localhost:8080/v1/chat"),
            "http://localhost:8080/v1/chat"
        );
        // Non-URL input passes through unchanged
        assert_eq!(redact_url("api.example.com"), "api.example.com");
        // Embedded URLs inside free-form error text are redacted in place
        let text =
            "error sending request for url (https://x.example.com/v1?q=SECRET) after 3 tries";
        assert_eq!(
            redact_urls_in(text),
            "error sending request for url (https://x.example.com/v1) after 3 tries"
        );
        assert!(
            redact_urls_in(text).contains("error sending request"),
            "重试逻辑依赖的错误子串必须保留"
        );
        // Text without URLs is untouched
        assert_eq!(redact_urls_in("plain error"), "plain error");
    }
}
