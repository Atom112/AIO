//! Unified HTTP URL safety validation.
//!
//! Provides the single `validate_http_url` function reused by every HTTP entry point
//! (LLM API, provider config, MCP HTTP transport, web fetch, etc.).
//!
//! # Validation rules
//!
//! - Non-empty + valid URL parse
//! - Scheme is `http` or `https` (HTTPS-only is configurable)
//! - Reject bare IP addresses (IPv4 / IPv6)
//! - Reject intranet / cloud metadata hostnames
//! - Whether localhost is allowed is configurable (local engines / MCP servers need it)
//!
//! # DNS Rebinding
//!
//! This module only performs static hostname string checks; it does no DNS resolution.
//! DNS rebinding attacks that map a domain to a private IP require OS- or network-layer defenses.
//!
//! # Usage
//!
//! ```ignore
//! use crate::utils::url_validation::{validate_http_url, HttpUrlOptions};
//!
//! // Provider API URL (remote APIs must not use localhost)
//! validate_http_url(&api_url, &HttpUrlOptions::default())?;
//!
//! // Local LLM engine URL (needs HTTP + localhost)
//! validate_http_url(&api_url, &HttpUrlOptions::local_engine())?;
//! ```

use crate::ierr;

/// Validation options.
pub struct HttpUrlOptions {
    /// Whether the HTTP protocol is allowed (`false` = HTTPS only).
    pub allow_http: bool,
    /// Whether localhost / 127.0.0.1 / [::1] is allowed.
    pub allow_localhost: bool,
}

impl Default for HttpUrlOptions {
    /// Default: HTTP allowed, localhost forbidden.
    /// Intended for remote provider API URL validation.
    fn default() -> Self {
        Self {
            allow_http: true,
            allow_localhost: false,
        }
    }
}

impl HttpUrlOptions {
    /// Preset: HTTPS only (for web fetch/search).
    pub fn https_only() -> Self {
        Self {
            allow_http: false,
            allow_localhost: false,
        }
    }

    /// Preset: HTTP + localhost allowed (for local engines / MCP servers).
    pub fn local_engine() -> Self {
        Self {
            allow_http: true,
            allow_localhost: true,
        }
    }
}

/// Cloud metadata endpoints (access is denied)
const CLOUD_METADATA_HOSTS: &[&str] = &["169.254.169.254", "metadata.google.internal"];

/// Validates the safety of an HTTP URL.
///
/// # Arguments
/// - `url_str` — the URL string to validate
/// - `options` — validation options
///
/// # Returns
/// `Ok(normalized_url)` — validation passed; returns the normalized URL string
/// `Err(reason)` — validation failed
pub fn validate_http_url(url_str: &str, options: &HttpUrlOptions) -> Result<String, String> {
    let trimmed = url_str.trim();
    if trimmed.is_empty() {
        return Err(ierr!("URL 不能为空", "URL must not be empty"));
    }

    // 1. URL parse
    let parsed = url::Url::parse(trimmed)
        .map_err(|e| ierr!("URL 解析失败: {}", "Failed to parse URL: {}", e))?;

    // 2. scheme check
    match parsed.scheme() {
        "https" => {}
        "http" if options.allow_http => {}
        "http" => return Err(ierr!("仅支持 HTTPS 协议", "Only HTTPS is allowed")),
        s => {
            return Err(ierr!(
                "不支持的协议: {}（仅允许 http/https）",
                "Unsupported protocol: {} (only http/https is allowed)",
                s
            ))
        }
    }

    // 3. host must exist
    let host = parsed
        .host_str()
        .ok_or_else(|| ierr!("URL 必须包含 host", "URL must contain a host"))?;

    // 4. Reject bare IP addresses (all IPv4 / IPv6)
    // Note: in url crate 2.x, host_str() returns IPv6 addresses in bracketed "[::1]" form
    let host_for_ip_check = host
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(host);
    if host_for_ip_check.parse::<std::net::Ipv4Addr>().is_ok()
        || host_for_ip_check.parse::<std::net::Ipv6Addr>().is_ok()
    {
        // localhost IP: allowed when allow_localhost is true
        if options.allow_localhost
            && (host_for_ip_check == "127.0.0.1" || host_for_ip_check == "::1")
        {
            return Ok(parsed.to_string());
        }
        return Err(ierr!(
            "不允许直接访问 IP 地址",
            "Direct access to IP addresses is not allowed"
        ));
    }

    let host_lower = host.to_lowercase();

    // 5. localhost hostname check
    if host_lower == "localhost" {
        if options.allow_localhost {
            return Ok(parsed.to_string());
        }
        return Err(ierr!(
            "不允许访问内网地址",
            "Access to intranet addresses is not allowed"
        ));
    }

    // 6. Cloud metadata endpoint blocking
    if CLOUD_METADATA_HOSTS.contains(&host_lower.as_str()) {
        return Err(ierr!(
            "不允许访问云 metadata 服务地址",
            "Access to cloud metadata service addresses is not allowed"
        ));
    }

    Ok(parsed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reject_empty() {
        assert!(validate_http_url("", &HttpUrlOptions::default()).is_err());
    }

    #[test]
    fn test_reject_invalid() {
        assert!(validate_http_url("not a url", &HttpUrlOptions::default()).is_err());
    }

    #[test]
    fn test_allow_https_external() {
        assert!(validate_http_url("https://api.openai.com", &HttpUrlOptions::default()).is_ok());
    }

    #[test]
    fn test_allow_http_when_permitted() {
        assert!(validate_http_url("http://example.com/api", &HttpUrlOptions::default()).is_ok());
    }

    #[test]
    fn test_reject_http_when_https_only() {
        assert!(validate_http_url("http://example.com", &HttpUrlOptions::https_only()).is_err());
    }

    #[test]
    fn test_reject_ip() {
        assert!(validate_http_url("http://192.168.1.1", &HttpUrlOptions::default()).is_err());
        assert!(validate_http_url("https://10.0.0.1/api", &HttpUrlOptions::default()).is_err());
    }

    #[test]
    fn test_reject_localhost_by_default() {
        assert!(validate_http_url("http://localhost:8080", &HttpUrlOptions::default()).is_err());
    }

    #[test]
    fn test_allow_localhost_when_permitted() {
        assert!(
            validate_http_url("http://localhost:8080", &HttpUrlOptions::local_engine()).is_ok()
        );
        assert!(
            validate_http_url("http://127.0.0.1:8080", &HttpUrlOptions::local_engine()).is_ok()
        );
    }

    #[test]
    fn test_reject_cloud_metadata() {
        assert!(validate_http_url(
            "http://169.254.169.254/latest/meta-data",
            &HttpUrlOptions::default()
        )
        .is_err());
        assert!(validate_http_url(
            "http://metadata.google.internal",
            &HttpUrlOptions::default()
        )
        .is_err());
    }

    #[test]
    fn test_reject_ipv6() {
        assert!(validate_http_url("http://[::1]:8080", &HttpUrlOptions::default()).is_err());
    }

    #[test]
    fn test_allow_ipv6_localhost_when_permitted() {
        assert!(validate_http_url("http://[::1]:8080", &HttpUrlOptions::local_engine()).is_ok());
    }
}
