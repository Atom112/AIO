//! 统一 HTTP URL 安全校验。
//!
//! 提供单一的 `validate_http_url` 函数，供所有 HTTP 入口点（LLM API、
//! Provider 配置、MCP HTTP transport、Web fetch 等）复用。
//!
//! # 校验规则
//!
//! - 非空 + 合法 URL parse
//! - scheme 为 `http` 或 `https`（可配置 HTTPS-only）
//! - 拒绝裸 IP 地址（IPv4 / IPv6）
//! - 拒绝内网/云 metadata hostname
//! - 可配置是否放行 localhost（本地引擎 / MCP server 需要）
//!
//! # DNS Rebinding
//!
//! 本模块仅做静态 hostname 字符串检查，不执行 DNS 解析。
//! 域名到私有 IP 的 DNS rebinding 攻击需 OS 或网络层防护。
//!
//! # 使用示例
//!
//! ```ignore
//! use crate::utils::url_validation::{validate_http_url, HttpUrlOptions};
//!
//! // Provider API URL（远程 API 不应使用 localhost）
//! validate_http_url(&api_url, &HttpUrlOptions::default())?;
//!
//! // 本地 LLM 引擎 URL（需要 HTTP + localhost）
//! validate_http_url(&api_url, &HttpUrlOptions::local_engine())?;
//! ```

/// 校验选项。
pub struct HttpUrlOptions {
    /// 是否允许 HTTP 协议（`false` = 仅 HTTPS）。
    pub allow_http: bool,
    /// 是否允许 localhost / 127.0.0.1 / [::1]。
    pub allow_localhost: bool,
}

impl Default for HttpUrlOptions {
    /// 默认配置：允许 HTTP，禁止 localhost。
    /// 适用于远程 Provider API URL 校验。
    fn default() -> Self {
        Self {
            allow_http: true,
            allow_localhost: false,
        }
    }
}

impl HttpUrlOptions {
    /// 预置：HTTPS only（用于 Web fetch/search）。
    pub fn https_only() -> Self {
        Self {
            allow_http: false,
            allow_localhost: false,
        }
    }

    /// 预置：允许 HTTP + localhost（用于本地引擎 / MCP server）。
    pub fn local_engine() -> Self {
        Self {
            allow_http: true,
            allow_localhost: true,
        }
    }
}

/// Cloud metadata 端点（拒绝访问）
const CLOUD_METADATA_HOSTS: &[&str] = &[
    "169.254.169.254",
    "metadata.google.internal",
];

/// 已知内网 hostname（localhost 变体除外，由 `allow_localhost` 控制）
const BLOCKED_HOSTNAMES: &[&str] = &[]; // 保留扩展空间

/// 校验 HTTP URL 的安全性。
///
/// # 参数
/// - `url_str` — 待校验的 URL 字符串
/// - `options` — 校验选项
///
/// # 返回
/// `Ok(normalized_url)` — 校验通过，返回规范化后的 URL 字符串
/// `Err(reason)` — 校验失败
pub fn validate_http_url(url_str: &str, options: &HttpUrlOptions) -> Result<String, String> {
    let trimmed = url_str.trim();
    if trimmed.is_empty() {
        return Err("URL 不能为空".into());
    }

    // 1. URL parse
    let parsed = url::Url::parse(trimmed).map_err(|e| format!("URL 解析失败: {e}"))?;

    // 2. scheme 检查
    match parsed.scheme() {
        "https" => {}
        "http" if options.allow_http => {}
        "http" => return Err("仅支持 HTTPS 协议".into()),
        s => return Err(format!("不支持的协议: {s}（仅允许 http/https）")),
    }

    // 3. host 必须存在
    let host = parsed.host_str().ok_or("URL 必须包含 host")?;

    // 4. 拒绝裸 IP 地址（所有 IPv4 / IPv6）
    // 注意：url crate 2.x 中 host_str() 对 IPv6 地址返回带括号的 "[::1]" 格式
    let host_for_ip_check = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')).unwrap_or(host);
    if host_for_ip_check.parse::<std::net::Ipv4Addr>().is_ok()
        || host_for_ip_check.parse::<std::net::Ipv6Addr>().is_ok()
    {
        // localhost IP 地址：若 allow_localhost 为 true，放行
        if options.allow_localhost
            && (host_for_ip_check == "127.0.0.1" || host_for_ip_check == "::1")
        {
            return Ok(parsed.to_string());
        }
        return Err("不允许直接访问 IP 地址".into());
    }

    let host_lower = host.to_lowercase();

    // 5. localhost 域名检查
    if host_lower == "localhost" {
        if options.allow_localhost {
            return Ok(parsed.to_string());
        }
        return Err("不允许访问内网地址".into());
    }

    // 6. Cloud metadata 端点拦截
    if CLOUD_METADATA_HOSTS.contains(&host_lower.as_str()) {
        return Err("不允许访问云 metadata 服务地址".into());
    }

    // 7. 自定义黑名单 hostname
    if BLOCKED_HOSTNAMES.contains(&host_lower.as_str()) {
        return Err("不允许访问该地址".into());
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
        assert!(validate_http_url("http://localhost:8080", &HttpUrlOptions::local_engine()).is_ok());
        assert!(validate_http_url("http://127.0.0.1:8080", &HttpUrlOptions::local_engine()).is_ok());
    }

    #[test]
    fn test_reject_cloud_metadata() {
        assert!(validate_http_url("http://169.254.169.254/latest/meta-data", &HttpUrlOptions::default()).is_err());
        assert!(validate_http_url("http://metadata.google.internal", &HttpUrlOptions::default()).is_err());
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