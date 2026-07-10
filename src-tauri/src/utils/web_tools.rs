//! 内置 Web 工具 — 网页抓取与搜索。
//!
//! 提供两个内置工具（非 MCP），供 Agent 在 run_agent_turn 中直接调用：
//! - `web_fetch`：抓取网页内容为纯文本
//! - `web_search`：通过 DuckDuckGo 搜索网页
//!
//! 安全措施：SSRF 防护（HTTPS-only + 内网 IP 屏蔽）、响应体大小限制、速率限制。

use crate::core::models::{ToolResult, ToolResultContent, ToolSpec, ToolFunctionSpec};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

// ====== 常量 ======

const MAX_BYTES: u64 = 1_000_000;       // 默认响应体上限 1MB
const FETCH_TIMEOUT: u64 = 30;           // web_fetch 总超时（秒）
const CONNECT_TIMEOUT: u64 = 5;          // 连接超时（秒）
const RATE_LIMIT_SECS: u64 = 3;          // web_search 最小间隔（秒）

// ====== 限流 ======

static LAST_SEARCH: Lazy<DashMap<(), Instant>> = Lazy::new(DashMap::new);

// ====== 辅助 ======

fn tool_ok(text: String) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": text}),
        }],
        is_error: false,
    }
}

fn tool_err(msg: &str) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": format!("[Error] {msg}")}),
        }],
        is_error: true,
    }
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(CONNECT_TIMEOUT))
        .timeout(Duration::from_secs(FETCH_TIMEOUT))
        .user_agent("AIO/0.6 (web_fetch; +https://github.com/Atom112/AIO)")
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .expect("构建 HTTP client 失败")
}

// ====== SSRF 防护 ======

/// 校验 URL 是否安全（HTTPS-only，非内网 IP）。
/// 仅做基础的主机名检查，不做 DNS 解析。
fn is_safe_url(url_str: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url_str).map_err(|e| format!("无效 URL: {e}"))?;
    if parsed.scheme() != "https" {
        return Err("仅支持 HTTPS 协议".into());
    }
    let host = parsed.host_str().unwrap_or("");
    // 屏蔽裸 IP（IPv4 / IPv6）
    if host.parse::<std::net::Ipv4Addr>().is_ok() || host.parse::<std::net::Ipv6Addr>().is_ok() {
        return Err("不允许直接访问 IP 地址".into());
    }
    // 屏蔽常见内网域名（防御 DNS rebinding）
    let blocked = ["localhost", "127.0.0.1", "[::1]"];
    if blocked.contains(&host.to_lowercase().as_str()) {
        return Err("不允许访问内网地址".into());
    }
    Ok(())
}

// ====== HTML → 文本 ======

/// 简单的 HTML 标签剥离，保留文字内容。
/// 不依赖第三方 HTML 解析库，适合 LLM 可读纯文本。
fn strip_html(html: &str) -> String {
    // 移除 script / style 标签及其内容
    let re_script = Regex::new(r"(?is)<script[^>]*>.*?</script>").unwrap();
    let re_style = Regex::new(r"(?is)<style[^>]*>.*?</style>").unwrap();
    let s = re_script.replace_all(html, "");
    let s = re_style.replace_all(&s, "");
    // 移除所有 HTML 标签
    let re_tag = Regex::new(r"<[^>]*>").unwrap();
    let s = re_tag.replace_all(&s, "");
    // 解码常见实体
    let s = s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    // 压缩多余空白行
    let re_ws = Regex::new(r"\n{3,}").unwrap();
    let result = re_ws.replace_all(&s, "\n\n").to_string();
    result.trim().to_string()
}

/// 截断文本到 max_bytes（字符级别），附加截断提示。
fn truncate(text: &str, max_bytes: u64) -> String {
    if text.len() as u64 <= max_bytes {
        return text.to_string();
    }
    let end = max_bytes as usize;
    // 尽量在换行处截断
    let cut = if let Some(pos) = text[..end].rfind('\n') {
        pos
    } else {
        end
    };
    format!("{}\n\n⚠ 内容过长已截断 ({}B / {}B)", &text[..cut], cut, max_bytes)
}

// ====== 工具定义 ======

pub fn get_web_tool_specs() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            kind: "function".into(),
            function: ToolFunctionSpec {
                name: "web_fetch".into(),
                description: "获取指定 URL 的网页内容，返回纯文本（HTML 标签已剥离）。仅支持 HTTPS。".into(),
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
                description: "通过 DuckDuckGo 搜索网页，返回结果摘要和链接。适用于查找最新文档、API 参考等。".into(),
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

// ====== 工具执行 ======

/// 执行 web_fetch：GET 请求指定 URL，返回纯文本。
pub async fn execute_web_fetch(url_str: &str, max_bytes: Option<u64>) -> ToolResult {
    // SSRF 校验
    if let Err(e) = is_safe_url(url_str) {
        return tool_err(&e);
    }
    let limit = max_bytes.unwrap_or(MAX_BYTES).min(5_000_000); // 硬上限 5MB

    let client = http_client();
    match client.get(url_str).send().await {
        Ok(resp) => {
            let status = resp.status();
            if !status.is_success() {
                return tool_err(&format!("HTTP {status}"));
            }
            let ct = resp.headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
            // 只处理 text/* 和 application/json
            let is_text = ct.starts_with("text/") || ct.contains("json") || ct.contains("xml") || ct.contains("javascript");
            let is_html = ct.contains("html");
            match resp.bytes().await {
                Ok(bytes) => {
                    if bytes.len() as u64 > limit {
                        return tool_err(&format!("响应体过大 ({}B > {limit}B)", bytes.len()));
                    }
                    let text = String::from_utf8_lossy(&bytes).to_string();
                    let result = if is_text && is_html {
                        strip_html(&text)
                    } else {
                        text
                    };
                    tool_ok(truncate(&result, limit))
                }
                Err(e) => tool_err(&format!("读取响应失败: {e}")),
            }
        }
        Err(e) => {
            if e.is_timeout() {
                tool_err("请求超时")
            } else if e.is_connect() {
                tool_err(&format!("无法连接: {e}"))
            } else {
                tool_err(&format!("请求失败: {e}"))
            }
        }
    }
}

/// 执行 web_search：DuckDuckGo Instant Answer API。
pub async fn execute_web_search(query: &str, count: Option<u64>) -> ToolResult {
    // 速率限制
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
                    // Abstract（摘要）
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
                            if n >= limit { break; }
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
                    // Results（常规搜索结果，当没有 RelatedTopics 时使用）
                    if lines.len() <= 1 {
                        if let Some(results) = data["Results"].as_array() {
                            let mut n = 0;
                            for r in results {
                                if n >= limit { break; }
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

/// URL 编码（DuckDuckGo API 使用标准 percent-encoding）
fn urlencoding(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
}
