//! Built-in web tools — page fetching and search.
//!
//! Provides two built-in (non-MCP) tools for the agent to call directly in run_agent_turn:
//! - `web_fetch`: fetch a page and return its content as plain text
//! - `web_search`: search the web via DuckDuckGo
//!
//! Safety: SSRF protection (HTTPS-only + private-IP blocking), response body size limit, rate limiting.

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolSpec};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use url::Url;

// ====== Constants ======

const MAX_BYTES: u64 = 1_000_000; // default response body cap: 1MB
const FETCH_TIMEOUT: u64 = 30; // web_fetch overall timeout (seconds)
const CONNECT_TIMEOUT: u64 = 5; // connect timeout (seconds)
const RATE_LIMIT_SECS: u64 = 3; // minimum interval between web_search calls (seconds)
const MAX_REDIRECTS: u32 = 5; // max redirects web_fetch follows manually

// ====== Rate limiting ======

static LAST_SEARCH: Lazy<DashMap<(), Instant>> = Lazy::new(DashMap::new);

// ====== Helpers ======

// tool_ok / tool_err are provided by utils::tool_result (single source of truth repo-wide)
use crate::utils::tool_result::{tool_err, tool_ok};

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT))
        .timeout(Duration::from_secs(FETCH_TIMEOUT))
        .user_agent("AIO/0.6 (web_fetch; +https://github.com/Atom112/AIO)")
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .expect("构建 HTTP client 失败")
}

/// web_fetch only: disables auto redirects in favor of hop-by-hop manual validation (per-hop SSRF checks).
fn http_client_no_redirect() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT))
        .timeout(Duration::from_secs(FETCH_TIMEOUT))
        .user_agent("AIO/0.6 (web_fetch; +https://github.com/Atom112/AIO)")
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("构建 HTTP client 失败")
}

/// SEC-03R: a one-shot no-redirect client whose DNS for `domain` is pinned to the
/// pre-validated `addr` (reqwest 0.13 exposes resolve() only on the builder, so
/// each fetch hop gets its own client; hops are few and one-request each).
fn http_client_no_redirect_pinned(domain: &str, addr: std::net::SocketAddr) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT))
        .timeout(Duration::from_secs(FETCH_TIMEOUT))
        .user_agent("AIO/0.6 (web_fetch; +https://github.com/Atom112/AIO)")
        .redirect(reqwest::redirect::Policy::none())
        .resolve(domain, addr)
        .build()
        .expect("构建 HTTP client 失败")
}

// ====== SSRF protection ======

/// Checks that a URL is safe (HTTPS-only, not an intranet IP).
/// Delegates to the unified URL validation module; static hostname check only, no DNS resolution.
fn is_safe_url(url_str: &str) -> Result<(), String> {
    crate::utils::url_validation::validate_http_url(
        url_str,
        &crate::utils::url_validation::HttpUrlOptions::https_only(),
    )
    .map(|_| ())
    // The unified validation module already returns zh-CN error messages; pass them through unchanged.
}

/// SEC-03R: whether a resolved address is safe to connect to (public unicast only).
fn is_public_addr(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4.is_documentation()
                // 0.0.0.0/8 ("this network") and carrier-grade NAT 100.64.0.0/10
                || o[0] == 0
                || (o[0] == 100 && (o[1] & 0xC0) == 64))
        }
        std::net::IpAddr::V6(v6) => {
            // Manual scope checks (Ipv6Addr::is_unicast_global is still unstable):
            // reject loopback ::1, unspecified ::, link-local fe80::/10,
            // unique-local fc00::/7, multicast ff00::/8; everything else counts as
            // global unicast (IPv4-mapped forms are handled by their mapped range).
            let s = v6.segments();
            !(v6.is_loopback()
                || v6.is_unspecified()
                || (s[0] & 0xffc0) == 0xfe80
                || (s[0] & 0xfe00) == 0xfc00
                || (s[0] & 0xff00) == 0xff00)
        }
    }
}

/// SEC-03R: resolve the URL's host and validate every resolved address.
///
/// The hostname-only check above cannot see DNS — a rebinding attacker maps a
/// benign-looking domain to a private/metadata IP (or flips the record between the
/// check and the connect). This resolves the host NOW and requires every address to
/// be public; the returned `(host, addr)` pair is pinned onto the request client via
/// `ClientBuilder::resolve`, so the connection uses exactly the validated IP and no
/// second DNS lookup can happen.
///
/// Returns `Ok(None)` when the host is an IP literal (already rejected by the
/// hostname check; nothing to pin).
async fn resolve_validated_addr(
    parsed: &Url,
) -> Result<Option<(String, std::net::SocketAddr)>, String> {
    let host_raw = parsed
        .host_str()
        .ok_or_else(|| "URL 缺少 host".to_string())?;
    let host = host_raw
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_string();
    // IP literal: validated by the string-level check already
    if host.parse::<std::net::Ipv4Addr>().is_ok() || host.parse::<std::net::Ipv6Addr>().is_ok() {
        return Ok(None);
    }
    let port = parsed
        .port()
        .unwrap_or(if parsed.scheme() == "https" { 443 } else { 80 });
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("域名解析失败: {e}"))?
        .collect();
    if addrs.is_empty() {
        return Err("域名没有可用的解析记录".to_string());
    }
    if let Some(bad) = addrs.iter().find(|a| !is_public_addr(a.ip())) {
        return Err(format!(
            "域名解析到内网/保留地址（{}），已拒绝访问",
            bad.ip()
        ));
    }
    Ok(addrs.into_iter().next().map(|addr| (host, addr)))
}

// ====== HTML → text ======

/// Simple HTML tag stripping that keeps the text content.
/// No third-party HTML parser dependency; output suits LLM-readable plain text.
fn strip_html(html: &str) -> String {
    // Remove script / style tags and their contents
    let re_script = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let re_style = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let s = re_script.replace_all(html, "");
    let s = re_style.replace_all(&s, "");
    // Remove all HTML tags
    let re_tag = Regex::new(r"<[^>]*>").unwrap();
    let s = re_tag.replace_all(&s, "");
    // Decode common HTML entities
    let s = s
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    // Collapse excess blank lines
    let re_ws = Regex::new(r"\n{3,}").unwrap();
    let result = re_ws.replace_all(&s, "\n\n").to_string();
    result.trim().to_string()
}

// Truncation is unified in utils::truncate (the function name states the semantics)
use crate::utils::truncate::truncate_bytes_head;

#[allow(dead_code)] // keep the alias: web_fetch still calls it with max_bytes(u64)
fn truncate(text: &str, max_bytes: u64) -> String {
    truncate_bytes_head(text, max_bytes as usize)
}

// ====== Tool definitions ======

pub fn get_web_tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "web_fetch".into(),
                description:
                    "获取指定 URL 的网页内容，返回纯文本（HTML 标签已剥离）。仅支持 HTTPS。".into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "url": { "type": "string", "description": "要获取的网页 URL（必须 https://）" },
                        "max_bytes": { "type": "integer", "description": "最大返回字节数，默认 1MB" }
                    },
                    "required": ["url"]
                }),
            },
        },
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "web_search".into(),
                description:
                    "通过 DuckDuckGo 搜索网页，返回结果摘要和链接。适用于查找最新文档、API 参考等。"
                        .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "搜索关键词" },
                        "count": { "type": "integer", "description": "返回结果数量（默认 5）" }
                    },
                    "required": ["query"]
                }),
            },
        },
    ]
}

// ====== Tool execution ======

/// Resolves a redirect Location (relative paths supported) to an absolute URL.
fn resolve_redirect(base: &str, location: &str) -> Option<String> {
    Url::parse(base)
        .ok()?
        .join(location)
        .ok()
        .map(|u| u.to_string())
}

/// Executes web_fetch: GET the given URL and return plain text.
/// Redirects: followed manually hop by hop, re-running SSRF validation at each hop.
/// SEC-03R: each hop resolves + validates the host's addresses and pins the
/// connection to the validated IP (see resolve_validated_addr).
pub async fn execute_web_fetch(url_str: &str, max_bytes: Option<u64>) -> ToolResult {
    let limit = max_bytes.unwrap_or(MAX_BYTES).min(5_000_000); // hard cap 5MB

    let mut current = url_str.to_string();

    for _hop in 0..=MAX_REDIRECTS {
        // Re-validate at every hop (entry URL + each redirect target)
        if let Err(e) = is_safe_url(&current) {
            return tool_err(&e);
        }
        // SEC-03R: resolve + validate the host's actual addresses, then pin the
        // request to the validated IP via a ClientBuilder-level DNS override —
        // closes both the "private-IP DNS record" bypass and the check-then-
        // connect rebinding window (reqwest 0.13 exposes resolve() only on the
        // builder, so each hop gets a one-shot pinned client).
        let parsed_current = match Url::parse(&current) {
            Ok(u) => u,
            Err(e) => return tool_err(&format!("URL 解析失败: {e}")),
        };
        let pinned = match resolve_validated_addr(&parsed_current).await {
            Ok(p) => p,
            Err(e) => return tool_err(&e),
        };
        let hop_client = match pinned {
            Some((host, addr)) => http_client_no_redirect_pinned(&host, addr),
            None => http_client_no_redirect(),
        };

        let resp = match hop_client.get(&current).send().await {
            Ok(r) => r,
            Err(e) => {
                if e.is_timeout() {
                    return tool_err("请求超时");
                } else if e.is_connect() {
                    return tool_err(&format!("无法连接: {e}"));
                } else {
                    return tool_err(&format!("请求失败: {e}"));
                }
            }
        };

        // 3xx redirect: resolve Location and continue to the next hop (re-validated)
        if resp.status().is_redirection() {
            let loc = match resp.headers().get("location").and_then(|v| v.to_str().ok()) {
                Some(l) => l,
                None => return tool_err("重定向缺少 Location 头"),
            };
            current = match resolve_redirect(&current, loc) {
                Some(u) => u,
                None => return tool_err("无法解析重定向地址"),
            };
            continue;
        }

        let status = resp.status();
        if !status.is_success() {
            return tool_err(&format!("HTTP {status}"));
        }
        let ct = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        // Only handle text/* and application/json
        let is_text = ct.starts_with("text/")
            || ct.contains("json")
            || ct.contains("xml")
            || ct.contains("javascript");
        let is_html = ct.contains("html");
        // SEC-08: never buffer the whole body. The declared Content-Length is checked
        // first, then the stream is drained chunk by chunk and aborted the moment the
        // limit is exceeded — a hostile/huge response cannot OOM the agent turn.
        if let Some(len) = resp.content_length() {
            if len > limit {
                return tool_err(&format!("响应体过大 (Content-Length {len}B > {limit}B)"));
            }
        }
        let mut resp = resp;
        let mut body: Vec<u8> = Vec::new();
        loop {
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    if body.len() as u64 + chunk.len() as u64 > limit {
                        return tool_err(&format!("响应体过大 (> {limit}B)"));
                    }
                    body.extend_from_slice(&chunk);
                }
                Ok(None) => break,
                Err(e) => return tool_err(&format!("读取响应失败: {e}")),
            }
        }
        let text = String::from_utf8_lossy(&body).to_string();
        let result = if is_text && is_html {
            strip_html(&text)
        } else {
            text
        };
        return tool_ok(truncate(&result, limit));
    }

    tool_err(&format!("重定向次数超过上限 ({MAX_REDIRECTS})"))
}

/// Executes web_search via the DuckDuckGo Instant Answer API.
pub async fn execute_web_search(query: &str, count: Option<u64>) -> ToolResult {
    // Rate limiting
    {
        let now = Instant::now();
        if let Some(entry) = LAST_SEARCH.get(&()) {
            let elapsed = now.duration_since(*entry.value()).as_secs();
            if elapsed < RATE_LIMIT_SECS {
                return tool_err(&format!(
                    "搜索频率过高，请 {} 秒后重试",
                    RATE_LIMIT_SECS - elapsed
                ));
            }
        }
        LAST_SEARCH.insert((), now);
    }

    let limit = count.unwrap_or(5).min(20) as usize;
    let url = format!(
        "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
        urlencoding(query)
    );

    let client = http_client();
    match client.get(&url).send().await {
        Ok(resp) => {
            if !resp.status().is_success() {
                return tool_err(&format!("搜索 API 返回 HTTP {}", resp.status()));
            }
            match resp.json::<Value>().await {
                Ok(data) => {
                    let mut lines: Vec<String> = Vec::new();
                    // Abstract (summary)
                    if let Some(abstract_text) = data["AbstractText"].as_str() {
                        if !abstract_text.is_empty() {
                            let source = data["AbstractURL"].as_str().unwrap_or("");
                            lines.push(format!("📌 {}", abstract_text));
                            if !source.is_empty() {
                                lines.push(format!("   来源: {source}"));
                            }
                            lines.push(String::new());
                        }
                    }
                    // RelatedTopics
                    if let Some(topics) = data["RelatedTopics"].as_array() {
                        let mut n = 0;
                        for topic in topics {
                            if n >= limit {
                                break;
                            }
                            if let Some(text) = topic["Text"].as_str() {
                                let url = topic["FirstURL"].as_str().unwrap_or("");
                                lines.push(format!("🔗 {}", text));
                                if !url.is_empty() {
                                    lines.push(format!("   {url}"));
                                }
                                n += 1;
                            }
                        }
                    }
                    // Results (regular search results, used when there are no RelatedTopics)
                    if lines.len() <= 1 {
                        if let Some(results) = data["Results"].as_array() {
                            let mut n = 0;
                            for r in results {
                                if n >= limit {
                                    break;
                                }
                                if let Some(text) = r["Text"].as_str() {
                                    let url = r["FirstURL"].as_str().unwrap_or("");
                                    lines.push(format!("🔗 {}", text));
                                    if !url.is_empty() {
                                        lines.push(format!("   {url}"));
                                    }
                                    n += 1;
                                }
                            }
                        }
                    }
                    if lines.is_empty() {
                        tool_ok(format!("未找到与 '{}' 相关的结果。", query))
                    } else {
                        tool_ok(format!("🔍 '{}' 搜索结果:\n\n{}", query, lines.join("\n")))
                    }
                }
                Err(e) => tool_err(&format!("解析搜索结果失败: {e}")),
            }
        }
        Err(e) => {
            if e.is_timeout() {
                tool_err("搜索请求超时")
            } else {
                tool_err(&format!("搜索请求失败: {e}"))
            }
        }
    }
}

/// URL-encodes a string (the DuckDuckGo API expects standard percent-encoding)
fn urlencoding(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SEC-03R: resolved-address validation must reject every private/reserved
    /// range (the hostname string check cannot see DNS at all).
    #[test]
    fn resolved_address_validation_rejects_private_ranges() {
        use std::net::IpAddr;
        let v4 = |a: u8, b: u8, c: u8, d: u8| IpAddr::from([a, b, c, d]);
        // Private / reserved: rejected
        for ip in [
            v4(10, 0, 0, 1),        // 10/8 private
            v4(172, 16, 0, 1),      // 172.16/12 private
            v4(192, 168, 1, 1),     // 192.168/16 private
            v4(127, 0, 0, 1),       // loopback
            v4(169, 254, 169, 254), // link-local / metadata
            v4(0, 0, 0, 0),         // "this network"
            v4(100, 64, 0, 1),      // carrier-grade NAT
            v4(224, 0, 0, 1),       // multicast
            v4(192, 0, 2, 1),       // TEST-NET (documentation)
            v4(255, 255, 255, 255), // broadcast
        ] {
            assert!(!is_public_addr(ip), "{ip} 应被拒绝");
        }
        // Public: allowed
        for ip in [v4(1, 1, 1, 1), v4(8, 8, 8, 8), v4(93, 184, 216, 34)] {
            assert!(is_public_addr(ip), "{ip} 应被放行");
        }
        // IPv6: loopback/link-local/unique-local rejected, global unicast allowed
        let v6 = |s: &str| s.parse::<std::net::Ipv6Addr>().unwrap();
        assert!(!is_public_addr(IpAddr::V6(v6("::1"))));
        assert!(!is_public_addr(IpAddr::V6(v6("fe80::1"))));
        assert!(!is_public_addr(IpAddr::V6(v6("fd00::1"))));
        assert!(is_public_addr(IpAddr::V6(v6("2606:4700::1111"))));
    }
}
