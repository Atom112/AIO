//! Built-in web tools — page fetching and search.
//!
//! Provides two built-in (non-MCP) tools for the agent to call directly in run_agent_turn:
//! - `web_fetch`: fetch a page and return its content as plain text
//! - `web_search`: search the web via Bing / DuckDuckGo
//!
//! Safety: SSRF protection (HTTPS-only + private-IP blocking), response body size limit, rate limiting.

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolSpec};
use base64::Engine;
use dashmap::DashMap;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use url::Url;

// ====== Constants ======

const MAX_BYTES: u64 = 1_000_000; // default response body cap: 1MB
const FETCH_TIMEOUT: u64 = 30; // web_fetch overall timeout (seconds)
const CONNECT_TIMEOUT: u64 = 5; // connect timeout (seconds)
const RATE_LIMIT_SECS: u64 = 3; // minimum interval between web_search calls (seconds)
const MAX_REDIRECTS: u32 = 5; // max redirects web_fetch follows manually
const SEARCH_HTML_CAP: u64 = 262_144; // web_search results page cap: 256KB (lite pages are ~25KB)

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

/// Browser-like UA for the search results pages. Search engines' bot mitigation
/// (Bing and DuckDuckGo alike) serves challenge/empty pages to non-browser
/// agents, so the search fetch identifies as a normal desktop browser.
const SEARCH_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Search-page client: browser-shaped UA + headers (a bare UA string is not
/// enough — mitigations also look at Accept/Accept-Language) and bounded
/// redirect following (Bing serves a region redirect before the results page).
fn search_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT))
        .timeout(Duration::from_secs(FETCH_TIMEOUT))
        .user_agent(SEARCH_USER_AGENT)
        .redirect(reqwest::redirect::Policy::limited(5))
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(
                reqwest::header::ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
                    .parse()
                    .expect("合法 Accept 头"),
            );
            h.insert(
                reqwest::header::ACCEPT_LANGUAGE,
                "zh-CN,zh;q=0.9,en;q=0.8"
                    .parse()
                    .expect("合法 Accept-Language 头"),
            );
            h
        })
        .build()
        .expect("构建搜索 HTTP client 失败")
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

// SEC-20: strip_html's regexes were recompiled on every web_fetch call —
// globally cached statics instead (regex compilation is ~100x the match cost).
static RE_SCRIPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap());
static RE_STYLE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap());
static RE_TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
static RE_BLANKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());

// web_search result parsing, two DuckDuckGo layouts:
// - html.duckduckgo.com/html/: `<a ... class="result__a" href="...">Title</a>`
//   + `<a class="result__snippet" ...>Snippet</a>` anchors in one result block
// - lite.duckduckgo.com/lite/: result rows alternate `<a ... class='result-link'>`
//   links and a following `<td class='result-snippet'>` cell
// (?s) guards against markup wrapping a title/snippet across lines.
// Bing: results are `<li class="b_algo">` blocks containing an `<h2><a href=…>`
// title link and a `<p>` snippet (block-scoped parse keeps favicon anchors out).
static RE_BING_BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<li class="b_algo"[^>]*>(.*?)</li>"#).unwrap());
static RE_BING_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<h2[^>]*><a[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#).unwrap());
static RE_BING_SNIPPET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<p[^>]*>(.*?)</p>"#).unwrap());
// DuckDuckGo HTML layout:
static RE_HTML_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<a[^>]*class="result__a"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#).unwrap()
});
static RE_HTML_SNIPPET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<a[^>]*class="result__snippet"[^>]*>(.*?)</a>"#).unwrap());
// DuckDuckGo Lite layout:
static RE_LITE_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<a[^>]*href="([^"]+)"[^>]*class='result-link'>(.*?)</a>"#).unwrap()
});
static RE_LITE_SNIPPET: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<td class='result-snippet'>(.*?)</td>"#).unwrap());
static RE_NUM_ENTITY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"&#x([0-9a-fA-F]{1,6});|&#([0-9]{1,8});").unwrap());

/// Simple HTML tag stripping that keeps the text content.
/// No third-party HTML parser dependency; output suits LLM-readable plain text.
fn strip_html(html: &str) -> String {
    // Remove script / style tags and their contents
    let s = RE_SCRIPT.replace_all(html, "");
    let s = RE_STYLE.replace_all(&s, "");
    // Remove all HTML tags
    let s = RE_TAG.replace_all(&s, "");
    // Decode common HTML entities
    let s = s
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    // Collapse excess blank lines
    let result = RE_BLANKS.replace_all(&s, "\n\n").to_string();
    result.trim().to_string()
}

// Truncation is unified in utils::truncate (the function name states the semantics)
use crate::utils::truncate::truncate_bytes_head;

#[allow(dead_code)] // keep the alias: web_fetch still calls it with max_bytes(u64)
fn truncate(text: &str, max_bytes: u64) -> String {
    truncate_bytes_head(text, max_bytes as usize)
}

// ====== web_search result parsing (DuckDuckGo HTML / Lite) ======

/// One parsed search hit from a DuckDuckGo results page.
#[derive(Debug, Default, PartialEq, Eq)]
struct SearchHit {
    title: String,
    url: String,
    snippet: String,
}

/// Removes HTML tags and collapses whitespace (titles/snippets carry inline
/// markup such as `<b>match terms</b>` and table indentation).
fn strip_tags_inline(s: &str) -> String {
    let no_tags = RE_TAG.replace_all(s, "");
    no_tags.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Decodes common HTML entities: named (`&amp;` etc., decoded last) plus numeric
/// forms (`&#x27;` hex / `&#39;` decimal) that the results markup uses for quotes.
fn decode_html_entities(s: &str) -> String {
    let named = s
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
        .replace("&amp;", "&");
    let decoded = RE_NUM_ENTITY.replace_all(&named, |caps: &regex::Captures| {
        let ch = if let Some(hex) = caps.get(1) {
            u32::from_str_radix(hex.as_str(), 16)
                .ok()
                .and_then(char::from_u32)
        } else {
            caps.get(2)
                .and_then(|d| d.as_str().parse::<u32>().ok())
                .and_then(char::from_u32)
        };
        ch.map(String::from).unwrap_or_default()
    });
    decoded.into_owned()
}

/// Extracts the real result URL from a search-results anchor href (Bing or
/// DuckDuckGo markup).
///
/// - DuckDuckGo: results arrive as redirect wrappers `//duckduckgo.com/l/?uddg=
///   <percent encoded real URL>&rut=...`; ads route through `y.js` links with no
///   `uddg` target.
/// - Bing: results are direct URLs, occasionally wrapped as
///   `https://www.bing.com/ck/a?...&u=a1<base64url real URL>`.
/// - Raw hrefs carry `&amp;` entities (decoded first).
///
/// Anything without an http(s) target is dropped (returns None).
fn extract_real_url(href: &str) -> Option<String> {
    let href = href.trim();
    // Decode entities first so parameter splitting works on raw separators
    let href = if href.contains('&') {
        href.replace("&amp;", "&")
    } else {
        href.to_string()
    };
    let href = href.strip_prefix("//").unwrap_or(&href);
    // DuckDuckGo ad redirect: no result target
    if href.starts_with("duckduckgo.com/y.js") {
        return None;
    }
    // DuckDuckGo redirect wrapper: uddg holds the percent-encoded target
    if let Some(pos) = href.find("uddg=") {
        let encoded = href[pos + "uddg=".len()..].split('&').next().unwrap_or("");
        let decoded = percent_encoding::percent_decode_str(encoded)
            .decode_utf8()
            .ok()?
            .trim()
            .to_string();
        if decoded.starts_with("http://") || decoded.starts_with("https://") {
            return Some(decoded);
        }
        return None;
    }
    // Bing redirect wrapper: u=a1<base64url> (the value arrives percent-encoded;
    // padding may or may not be present — strip it and use the no-pad alphabet)
    if href.starts_with("https://www.bing.com/ck/") || href.starts_with("http://www.bing.com/ck/") {
        let u = href
            .split(['?', '&'])
            .find(|p| p.starts_with("u="))
            .map(|p| &p["u=".len()..])?;
        let u = percent_encoding::percent_decode_str(u).decode_utf8().ok()?;
        let b64: String = u
            .strip_prefix("a1")
            .unwrap_or(&u)
            .chars()
            .filter(|c| *c != '=')
            .collect();
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(b64.as_bytes())
            .ok()?;
        let url = String::from_utf8(decoded).ok()?;
        if url.starts_with("http://") || url.starts_with("https://") {
            return Some(url);
        }
        return None;
    }
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    None
}

/// Generic row-pairing parser shared by both DuckDuckGo result layouts:
/// each result's title link is paired with the first snippet appearing after
/// it in document order. `links` rows are (link match end, href, title).
fn pair_links_with_snippets(
    links: Vec<(usize, &str, &str)>,
    snippets: Vec<(usize, &str)>,
) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    for (link_end, href, title) in links {
        let Some(url) = extract_real_url(href) else {
            continue; // ad / internal link
        };
        let title = decode_html_entities(&strip_tags_inline(title));
        let snippet = snippets
            .iter()
            .find(|(start, _)| *start > link_end)
            .map(|(_, text)| decode_html_entities(&strip_tags_inline(text)))
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        hits.push(SearchHit {
            title,
            url,
            snippet,
        });
    }
    hits
}

/// Parses a DuckDuckGo HTML results page (`html.duckduckgo.com/html/`):
/// titles are `<a ... class="result__a" href="...">` and snippets
/// `<a class="result__snippet" ...>` anchors inside one result block.
fn parse_ddg_html_results(html: &str) -> Vec<SearchHit> {
    let links: Vec<(usize, &str, &str)> = RE_HTML_LINK
        .captures_iter(html)
        .filter_map(|c| {
            let (whole, href, title) = (c.get(0)?, c.get(1)?, c.get(2)?);
            Some((whole.end(), href.as_str(), title.as_str()))
        })
        .collect();
    let snippets: Vec<(usize, &str)> = RE_HTML_SNIPPET
        .captures_iter(html)
        .filter_map(|c| {
            let (whole, text) = (c.get(0)?, c.get(1)?);
            Some((whole.start(), text.as_str()))
        })
        .collect();
    pair_links_with_snippets(links, snippets)
}

/// Parses a DuckDuckGo Lite results page (`lite.duckduckgo.com/lite/`):
/// result rows alternate `<a ... class='result-link'>` links and a following
/// `<td class='result-snippet'>` cell.
fn parse_ddg_lite_results(html: &str) -> Vec<SearchHit> {
    let links: Vec<(usize, &str, &str)> = RE_LITE_LINK
        .captures_iter(html)
        .filter_map(|c| {
            let (whole, href, title) = (c.get(0)?, c.get(1)?, c.get(2)?);
            Some((whole.end(), href.as_str(), title.as_str()))
        })
        .collect();
    let snippets: Vec<(usize, &str)> = RE_LITE_SNIPPET
        .captures_iter(html)
        .filter_map(|c| {
            let (whole, text) = (c.get(0)?, c.get(1)?);
            Some((whole.start(), text.as_str()))
        })
        .collect();
    pair_links_with_snippets(links, snippets)
}

/// Parses a Bing results page (`www.bing.com/search`):
/// each `<li class="b_algo">` block holds an `<h2><a href=…>` title link and a
/// first `<p>` snippet. Ad blocks (`b_ad` / aclick hrefs) and blocks without an
/// h2 link are skipped.
fn parse_bing_results(html: &str) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    for block in RE_BING_BLOCK.captures_iter(html) {
        let Some(body) = block.get(1) else {
            continue;
        };
        let body = body.as_str();
        // Skip ad blocks outright
        if body.contains("b_ad") {
            continue;
        }
        let Some((_, href, title)) = RE_BING_LINK
            .captures(body)
            .and_then(|c| Some((c.get(0)?, c.get(1)?, c.get(2)?)))
        else {
            continue;
        };
        let Some(url) = extract_real_url(href.as_str()) else {
            continue; // ad / internal link
        };
        if url.starts_with("https://www.bing.com/aclick")
            || url.starts_with("http://www.bing.com/aclick")
        {
            continue; // Bing ad redirect
        }
        let title = decode_html_entities(&strip_tags_inline(title.as_str()));
        if title.is_empty() {
            continue;
        }
        let snippet = RE_BING_SNIPPET
            .captures(body)
            .and_then(|c| c.get(1))
            .map(|text| decode_html_entities(&strip_tags_inline(text.as_str())))
            .unwrap_or_default();
        hits.push(SearchHit {
            title,
            url,
            snippet,
        });
    }
    hits
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
                    "搜索网页，返回结果摘要和链接（Bing 优先，DuckDuckGo 兜底）。适用于查找最新文档、API 参考等。"
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
                return tool_err(&format!(
                    "响应体超出 max_bytes 读取上限（{len}B > {limit}B）：这是本次调用设定的上限，不是页面问题；需要完整内容时调大 max_bytes 重试"
                ));
            }
        }
        let mut resp = resp;
        let mut body: Vec<u8> = Vec::new();
        loop {
            match resp.chunk().await {
                Ok(Some(chunk)) => {
                    if body.len() as u64 + chunk.len() as u64 > limit {
                        return tool_err(&format!(
                            "响应体超出 max_bytes 读取上限（已读 > {limit}B）：这是本次调用设定的上限，不是页面问题；需要完整内容时调大 max_bytes 重试"
                        ));
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

/// Executes web_search.
///
/// Chain: Bing results page → DuckDuckGo HTML results page → DuckDuckGo Lite
/// results page → DuckDuckGo Instant Answer API.
///
/// Bing is primary: it tolerates plain HTTP clients (no TLS/HTTP2 fingerprint
/// challenges like DuckDuckGo's) and is reachable without a proxy from mainland
/// China. The two DuckDuckGo result pages are keylessly scraped fallbacks
/// (independently mitigated, so one being challenge-blocked rarely blocks the
/// other). The Instant Answer API is a topic-abstract lookup, NOT a search
/// backend: it returns an empty payload for the vast majority of queries —
/// which is why web_search previously reported "未找到相关结果" for nearly every
/// search when it was the only source.
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

    // Source 1: Bing (primary — plain-HTTP-client friendly, no proxy needed in CN)
    match fetch_search_page("https://www.bing.com/search?q=", query, Some(limit)).await {
        Ok(html) => {
            let hits = parse_bing_results(&html);
            if !hits.is_empty() {
                return format_search_results(query, &hits, limit);
            }
            tracing::warn!("[web_search] Bing 未解析到结果，尝试 DuckDuckGo html 端点");
        }
        Err(e) => {
            tracing::warn!("[web_search] Bing 请求失败，尝试 DuckDuckGo html 端点: {e}");
        }
    }

    // Source 2: html.duckduckgo.com/html/
    match fetch_search_page("https://html.duckduckgo.com/html/?q=", query, None).await {
        Ok(html) => {
            let hits = parse_ddg_html_results(&html);
            if !hits.is_empty() {
                return format_search_results(query, &hits, limit);
            }
            // Page fetched but zero parsed hits: rare query or a bot-challenge
            // page — try the next source.
            tracing::warn!("[web_search] html 端点未解析到结果，尝试 lite 端点");
        }
        Err(e) => {
            tracing::warn!("[web_search] html 端点请求失败，尝试 lite 端点: {e}");
        }
    }

    // Source 3: lite.duckduckgo.com/lite/ (independently mitigated fallback)
    match fetch_search_page("https://lite.duckduckgo.com/lite/?q=", query, None).await {
        Ok(html) => {
            let hits = parse_ddg_lite_results(&html);
            if !hits.is_empty() {
                return format_search_results(query, &hits, limit);
            }
            tracing::warn!("[web_search] lite 端点未解析到结果，回退 Instant Answer API");
        }
        Err(e) => {
            tracing::warn!("[web_search] lite 端点请求失败，回退 Instant Answer API: {e}");
        }
    }

    // Source 4: Instant Answer API (topic abstracts; empty for most queries)
    instant_answer_search(query, limit).await
}

/// Formats parsed hits into the tool result text.
fn format_search_results(query: &str, hits: &[SearchHit], limit: usize) -> ToolResult {
    let lines: Vec<String> = hits
        .iter()
        .take(limit)
        .map(|h| {
            let mut line = format!("🔗 {}", h.title);
            if !h.url.is_empty() {
                line.push_str(&format!("\n   {}", h.url));
            }
            if !h.snippet.is_empty() {
                line.push_str(&format!("\n   {}", h.snippet));
            }
            line
        })
        .collect();
    tool_ok(format!(
        "🔍 '{}' 搜索结果:\n\n{}",
        query,
        lines.join("\n\n")
    ))
}

/// Shared search-page fetch (SEC-08-style bounded stream read; the fixed hosts
/// need no per-request SSRF pinning): browser-shaped UA + headers (bot
/// mitigation filters non-browser agents), redirect following (Bing serves a
/// region redirect first), bounded body read.
async fn fetch_search_page(
    base: &str,
    query: &str,
    count: Option<usize>,
) -> Result<String, String> {
    let mut url = format!("{}{}", base, urlencoding(query));
    if let Some(n) = count {
        url.push_str(&format!("&count={n}"));
    }
    let client = search_client();
    let resp = client.get(&url).send().await.map_err(|e| {
        if e.is_timeout() {
            "搜索请求超时".to_string()
        } else {
            format!("搜索请求失败: {e}")
        }
    })?;
    if !resp.status().is_success() {
        return Err(format!("搜索端点返回 HTTP {}", resp.status()));
    }
    let mut resp = resp;
    let mut body: Vec<u8> = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) => {
                if body.len() as u64 + chunk.len() as u64 > SEARCH_HTML_CAP {
                    return Err(format!("搜索响应过大 (> {SEARCH_HTML_CAP}B)"));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(e) => return Err(format!("读取搜索响应失败: {e}")),
        }
    }
    Ok(String::from_utf8_lossy(&body).to_string())
}

/// Fallback path: the DuckDuckGo Instant Answer API (abstract + related topics
/// for well-known topics only — most queries return nothing here).
async fn instant_answer_search(query: &str, limit: usize) -> ToolResult {
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

    /// web_search fix regression: the Bing results page must parse into (title,
    /// real URL, snippet) hits — `b_algo` blocks yield their h2 link + first <p>,
    /// `<strong>` highlights are stripped from titles, `/ck/a` base64url redirect
    /// wrappers decode, ad blocks are dropped.
    #[test]
    fn bing_results_parse_end_to_end() {
        let html = r#"
<li class="b_algo" data-id iid=SERP.5337>
  <div class="b_tpcn"><a class="tilk" href="https://www.python.org/downloads/" tabindex="-1"><cite>https://www.python.org › downloads</cite></a></div>
  <h2><a target="_blank" href="https://www.python.org/downloads/" h="ID=SERP,5142.2">Download <strong>Python</strong> | <strong>Python</strong>.org</a></h2>
  <div class="b_caption"><p class="b_lineclamp2">Starting with the Python 3.11.0 releases, CPython artifacts are signed with Sigstore. See …</p></div>
</li>
<li class="b_algo" data-id iid=SERP.5338>
  <h2><a target="_blank" href="https://www.bing.com/ck/a?lm=0&amp;u=a1aHR0cHM6Ly9kb2NzLnB5dGhvbi5vcmcvemgtY24vMy8%3D">Python 教程 — <strong>Python</strong> 3.15.0 文档</a></h2>
  <div class="b_caption"><p class="b_lineclamp2">The Python tutorial &#x27;hands&#39; on.</p></div>
</li>
<li class="b_ad" data-id iid=SERP.5339>
  <h2><a href="https://www.bing.com/aclick?ld=e8...">广告：Python 培训课程</a></h2>
  <p>买课送资料</p>
</li>
<li class="b_algo" data-id iid=SERP.5340>
  <div class="b_caption">no h2 link here</div>
</li>
"#;
        let hits = parse_bing_results(html);
        assert_eq!(hits.len(), 2, "广告块与无 h2 块应被过滤");
        assert_eq!(hits[0].url, "https://www.python.org/downloads/");
        assert_eq!(hits[0].title, "Download Python | Python.org");
        assert_eq!(
            hits[0].snippet,
            "Starting with the Python 3.11.0 releases, CPython artifacts are signed with Sigstore. See …"
        );
        assert_eq!(hits[1].url, "https://docs.python.org/zh-cn/3/");
        assert_eq!(hits[1].title, "Python 教程 — Python 3.15.0 文档");
        assert!(
            hits[1].snippet.contains("'hands'"),
            "数字实体应解码: {}",
            hits[1].snippet
        );
    }

    /// web_search fix regression: the DuckDuckGo HTML results page must parse into
    /// (title, real URL, snippet) hits — the `result__a`/`result__snippet` anchors
    /// pair by document order, redirect-wrapped links decode their `uddg` target,
    /// and ad (`y.js`) links are dropped.
    #[test]
    fn ddg_html_results_parse_end_to_end() {
        let html = r#"
<div class="links_main links_deep result__body">
  <h2 class="result__title">
    <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.python.org%2Fdoc%2F&amp;rut=abc123">Our Documentation | Python.org</a>
  </h2>
  <div class="result__extras">
    <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.python.org%2Fdoc%2F&amp;rut=abc123">The <b>official</b> home of the Python Programming Language</a>
  </div>
</div>
<div class="links_main links_deep result__body">
  <h2 class="result__title">
    <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdocs.python.org%2F3%2Ftutorial%2Findex.html&amp;rut=def456">Python 教程 — Python 3.15.0 文档</a>
  </h2>
  <a class="result__snippet" href="x">The Python tutorial &#x27;hands&#39; on.</a>
</div>
<div class="links_main links_deep result__body">
  <h2 class="result__title">
    <a rel="nofollow" class="result__a" href="//duckduckgo.com/y.js?ad_provider=b&amp;u2=https%3A%2F%2Fad.example%2F">Sponsored: Learn Python Fast</a>
  </h2>
  <a class="result__snippet" href="x">Buy our course now!</a>
</div>
"#;
        let hits = parse_ddg_html_results(html);
        assert_eq!(hits.len(), 2, "广告链接（y.js）应被过滤");
        assert_eq!(hits[0].url, "https://www.python.org/doc/");
        assert_eq!(hits[0].title, "Our Documentation | Python.org");
        assert_eq!(
            hits[0].snippet,
            "The official home of the Python Programming Language"
        );
        assert_eq!(hits[1].url, "https://docs.python.org/3/tutorial/index.html");
        assert_eq!(hits[1].title, "Python 教程 — Python 3.15.0 文档");
        assert!(
            hits[1].snippet.contains("'hands'"),
            "数字实体应解码: {}",
            hits[1].snippet
        );
    }

    /// web_search fix regression: the DuckDuckGo Lite results page must parse into
    /// (title, real URL, snippet) hits — redirect-wrapped links decode their `uddg`
    /// target, direct links pass through, ad (`y.js`) links are dropped.
    #[test]
    fn ddg_lite_results_parse_end_to_end() {
        let html = r#"
<table>
<tr><td><a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.python.org%2Fdoc%2F&amp;rut=abc123" class='result-link'>Our Documentation | Python.org</a></td></tr>
<tr><td>&nbsp;&nbsp;&nbsp;</td><td class='result-snippet'>Browse the docs online or <b>download</b> a copy of your own.</td></tr>
<tr><td><a rel="nofollow" href="https://docs.python.org/zh-cn/3/tutorial/index.html" class='result-link'>Python 教程 — Python 3.15.0 文档</a></td></tr>
<tr><td>&nbsp;</td><td class='result-snippet'>The Python tutorial &#x27;hands&#39; on.</td></tr>
<tr><td><a rel="nofollow" href="//duckduckgo.com/y.js?ad_provider=b&amp;u2=https%3A%2F%2Fad.example%2F" class='result-link'>Sponsored: Learn Python Fast</a></td></tr>
<tr><td>&nbsp;</td><td class='result-snippet'>Buy our course now!</td></tr>
</table>
"#;
        let hits = parse_ddg_lite_results(html);
        assert_eq!(hits.len(), 2, "广告链接（y.js）应被过滤");
        assert_eq!(hits[0].url, "https://www.python.org/doc/");
        assert_eq!(hits[0].title, "Our Documentation | Python.org");
        assert_eq!(
            hits[0].snippet,
            "Browse the docs online or download a copy of your own."
        );
        assert_eq!(
            hits[1].url,
            "https://docs.python.org/zh-cn/3/tutorial/index.html"
        );
        assert_eq!(hits[1].title, "Python 教程 — Python 3.15.0 文档");
        assert!(
            hits[1].snippet.contains("'hands'"),
            "数字实体应解码: {}",
            hits[1].snippet
        );
    }

    /// Zero results on an empty/challenge page must parse to an empty hit list
    /// (the caller then falls back to the Instant Answer API).
    #[test]
    fn ddg_lite_empty_page_parses_to_empty() {
        let hits = parse_ddg_lite_results("<html><body>anomaly challenge</body></html>");
        assert!(hits.is_empty());
    }

    /// URL extraction: redirect wrappers decode the `uddg` target (raw href carries
    /// `&amp;` entities), direct http(s) links pass through, internal links drop.
    #[test]
    fn extract_real_url_filters_internal_links() {
        assert_eq!(
            extract_real_url(
                "//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fa%3Fq%3D1&amp;rut=x"
            )
            .as_deref(),
            Some("https://example.com/a?q=1")
        );
        assert_eq!(
            extract_real_url("https://example.com/b").as_deref(),
            Some("https://example.com/b")
        );
        assert!(extract_real_url("//duckduckgo.com/y.js?ad_provider=b&u2=x").is_none());
        assert!(extract_real_url("//duckduckgo.com/?q=more").is_none());
    }

    /// Named + numeric (hex and decimal) entities decode; `&amp;` decodes last.
    #[test]
    fn html_entities_decode() {
        assert_eq!(
            decode_html_entities("&#x27;hi&#39; &amp; &quot;w&quot; &lt;b&gt;"),
            "'hi' & \"w\" <b>"
        );
    }
}
