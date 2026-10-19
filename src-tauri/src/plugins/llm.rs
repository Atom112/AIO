//! Shared LLM HTTP client and chat completion helpers - built on top of the provider plugins.
//!
//! Shared by commands/llm (main streaming loop) and services/memory (extract/judge/rewrite);
//! lives in the plugins layer: depends on provider plugins for Anthropic protocol adaptation, with no dependency on the commands layer.

use std::sync::LazyLock;

/// (PERF 审查修复) 全局单例：`reqwest::Client` 持连接池，重建即丢池——此前每次
/// LLM 调用（每回合主调用 / 标题 / 记忆管线 / 压缩 / fetch_models）都重建 Client，
/// 每个请求重付 TCP+TLS 握手。Client 内部是 Arc，clone 为零拷贝句柄。
/// 系统代理环境变量因此在首次使用时解析一次（桌面应用可接受：改代理需重启）。
static STREAMING_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        // Self-identifying UA (OpenCode Go rejects generic SDK/library agents;
        // some gateways key routing on it). Format matches build_http_client.
        .user_agent(format!("AIO-Desktop/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(600))
        .tcp_keepalive(std::time::Duration::from_secs(30))
        .http2_keep_alive_interval(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
});

/// Non-streaming client singleton (same reuse rationale as [`STREAMING_CLIENT`]).
static NON_STREAMING_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .user_agent(format!("AIO-Desktop/{}", env!("CARGO_PKG_VERSION")))
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
});

/// Streaming HTTP client (dedicated to LLM inference).
///
/// - connect_timeout(5s): quickly detect unreachable servers
/// - timeout(600s): 10-minute hard cap, preventing extremely slow inference from streaming forever
/// - tcp_keepalive(30s): detect TCP-level network partitions
/// - http2_keep_alive_interval(30s): detect silently dropped HTTP/2 connections
///
/// Chunk-level inactivity timeout (120s) is handled separately in the `stream_one_round()` loop.
pub(crate) fn streaming_http_client() -> reqwest::Client {
    STREAMING_CLIENT.clone()
}

/// Non-streaming HTTP client (summarize / generate_title / fetch_models).
///
/// Short requests should have short timeouts; 60s is ample for non-streaming LLM calls.
pub(crate) fn non_streaming_http_client() -> reqwest::Client {
    NON_STREAMING_CLIENT.clone()
}

// ===== SEC-7: per-origin pinned clients for the chat hot path =====
//
// The chat path carries the user's real API key on every request. `url_validation`
// is static (its module doc records the DNS-rebinding gap), so a provider domain
// could resolve benign at one moment and to a metadata/internal IP the next.
// `utils::ssrf` resolves + judges NOW and hands back a pin pair; the client below
// pins the connection to that validated address via `ClientBuilder::resolve`.
//
// Cache shape: the Client-per-origin reuse from the perf round is preserved — the
// pinned client is cached by (flavor, origin) and re-resolved only after the TTL,
// so a provider's legitimate DNS changes still propagate (within the TTL) while a
// mid-stream rebinding flip is impossible. Policy is AllowLocalNoMetadata: local
// engines on localhost are legitimate, metadata ranges never are.

/// Re-resolve (and rebuild) a pinned client at most this often.
const PIN_TTL: std::time::Duration = std::time::Duration::from_secs(600);

struct PinnedClientEntry {
    client: reqwest::Client,
    resolved_at: std::time::Instant,
}

static PINNED_CLIENTS: LazyLock<
    parking_lot::Mutex<std::collections::HashMap<String, PinnedClientEntry>>,
> = LazyLock::new(|| parking_lot::Mutex::new(std::collections::HashMap::new()));

/// Resolve the URL, judge it under the user-configured-endpoint policy, and return
/// the client to use. `Ok(client)` on every path the request may proceed on;
/// `Err` ONLY for policy violations, which the caller must surface as the request
/// failure (the request must not go out).
async fn pinned_client_for(api_url: &str, streaming: bool) -> Result<reqwest::Client, String> {
    use crate::utils::ssrf::{resolve_pinned, ConnectPolicy, ResolveOutcome};

    let parsed = match url::Url::parse(api_url) {
        Ok(p) => p,
        // Unparseable: let the request fail with reqwest's own error (unchanged behavior)
        Err(_) => {
            return Ok(if streaming {
                streaming_http_client()
            } else {
                non_streaming_http_client()
            })
        }
    };
    let host = parsed.host_str().unwrap_or_default().to_string();
    if host.is_empty() {
        return Ok(if streaming {
            streaming_http_client()
        } else {
            non_streaming_http_client()
        });
    }
    let origin = match parsed.port() {
        Some(p) => format!("{host}:{p}"),
        None => host,
    };
    let cache_key = format!("{}|{}", if streaming { "s" } else { "n" }, origin);

    // Fresh entry → reuse without touching DNS (hot path: zero cost beyond one map lookup)
    if let Some(entry) = PINNED_CLIENTS.lock().get(&cache_key) {
        if entry.resolved_at.elapsed() < PIN_TTL {
            return Ok(entry.client.clone());
        }
    }

    match resolve_pinned(&parsed, ConnectPolicy::AllowLocalNoMetadata).await {
        ResolveOutcome::LiteralOk => Ok(base_client(streaming)),
        ResolveOutcome::DnsUnavailable(m) => {
            // Offline / resolver failure: keep connectivity semantics identical
            // (the connect fails the same way). A stale pinned entry still wins —
            // its address was validated when pinned.
            tracing::warn!("[llm] 域名解析失败，使用未钉扎连接: {m}");
            if let Some(entry) = PINNED_CLIENTS.lock().get(&cache_key) {
                Ok(entry.client.clone())
            } else {
                Ok(base_client(streaming))
            }
        }
        ResolveOutcome::Rejected(m) => Err(m),
        ResolveOutcome::Pinned {
            host: pin_host,
            addr,
        } => {
            let mut builder = if streaming {
                reqwest::Client::builder()
                    .user_agent(format!("AIO-Desktop/{}", env!("CARGO_PKG_VERSION")))
                    .connect_timeout(std::time::Duration::from_secs(5))
                    .timeout(std::time::Duration::from_secs(600))
                    .tcp_keepalive(std::time::Duration::from_secs(30))
                    .http2_keep_alive_interval(std::time::Duration::from_secs(30))
            } else {
                reqwest::Client::builder()
                    .user_agent(format!("AIO-Desktop/{}", env!("CARGO_PKG_VERSION")))
                    .connect_timeout(std::time::Duration::from_secs(5))
                    .timeout(std::time::Duration::from_secs(60))
            };
            builder = builder.resolve(&pin_host, addr);
            let client = builder.build().unwrap_or_else(|_| reqwest::Client::new());
            PINNED_CLIENTS.lock().insert(
                cache_key,
                PinnedClientEntry {
                    client: client.clone(),
                    resolved_at: std::time::Instant::now(),
                },
            );
            Ok(client)
        }
    }
}

fn base_client(streaming: bool) -> reqwest::Client {
    if streaming {
        streaming_http_client()
    } else {
        non_streaming_http_client()
    }
}

/// Streaming flavor of the pinned accessor (see the SEC-7 block above).
pub(crate) async fn streaming_http_client_pinned(api_url: &str) -> Result<reqwest::Client, String> {
    pinned_client_for(api_url, true).await
}

/// Non-streaming flavor of the pinned accessor (see the SEC-7 block above).
pub(crate) async fn non_streaming_http_client_pinned(
    api_url: &str,
) -> Result<reqwest::Client, String> {
    pinned_client_for(api_url, false).await
}

/// Sends a chat completion request — 阶段4 第5项（聊天路径统一出口）: the provider
/// plugin system owns the whole request shape. `chat_completions_url` builds the
/// endpoint, `apply_chat_auth` attaches auth (Bearer by default; Anthropic its
/// native headers; Ollama none — the historic two hardcoded branches and the
/// second host-matching call are gone), and the Anthropic native protocol adapts
/// the body when the dispatched plugin is Anthropic. `apply_chat_session`
/// attaches conversation-scoped session identity afterward (OpenCode Go's
/// `x-opencode-session`; no-op for every other provider).
///
/// `session_id`: the conversation identity (AIO topic id) when the caller has
/// one — conversation-bearing callers MUST pass it (OpenCode Go rejects
/// session-less chat requests). `None` is for conversation-less auxiliary calls
/// (title generation / BTW / memory pipeline); plugins substitute a stable
/// per-process session.
///
/// Returns (response, is_anthropic); the caller parses the response accordingly.
pub(crate) async fn post_chat_completion(
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    body: &serde_json::Value,
    session_id: Option<&str>,
) -> Result<(reqwest::Response, bool), String> {
    let plugin = crate::plugins::provider::ProviderManager::new().for_url(api_url);
    let endpoint = plugin.chat_completions_url(api_url);
    let is_anthropic = plugin.identifier() == "anthropic";
    // (PERF 审查修复) 非 Anthropic 路径直接借用原 body 序列化——此前无条件
    // `body.clone()` 对整个消息数组（长会话数百 KB）做一次深拷贝，纯属浪费；
    // 仅 Anthropic 分支需要转换出新值。
    let req = if is_anthropic {
        let request_body = crate::plugins::provider::anthropic::to_anthropic_body(body)?;
        plugin.apply_chat_auth(client.post(&endpoint).json(&request_body), api_key)
    } else {
        plugin.apply_chat_auth(client.post(&endpoint).json(body), api_key)
    };
    let req = plugin.apply_chat_session(req, session_id);
    let res = req
        .send()
        .await
        // PROV-11: reqwest's Display carries the full URL (Gemini-style auth puts
        // the key in ?key=...) — redact before the text reaches toasts/logs.
        // The "error sending request …" prefix is preserved for retry detection.
        .map_err(|e| crate::plugins::provider::redact_urls_in(&e.to_string()))?;
    Ok((res, is_anthropic))
}

/// Whether an HTTP status is worth retrying (rate limiting + transient gateway/server faults).
fn is_retryable_http_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504 | 529)
}

/// Whether a transport-layer error is retryable. This wrapper only covers the POST send phase (no streaming
/// content emitted yet), so every connection error here is safe to retry.
fn is_retryable_transport_error(e: &str) -> bool {
    let lower = e.to_lowercase();
    lower.contains("error sending request")
        || lower.contains("connection")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("dns")
        || lower.contains("tls")
        || lower.contains("broken pipe")
}

/// Pre-stream retry for main-model calls (POST + HTTP status check).
///
/// Covers only the phase before any streaming emission, so retries never duplicate frontend content. Mid-stream
/// errors (120s inactivity / chunk read failure / user cancel) are not retried here; upper-level teardown handles them.
/// Transport errors and 408/429/5xx statuses retry with 1s/2s/4s + jitter backoff, capped at max_retries.
pub(crate) async fn post_chat_completion_with_retry(
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    body: &serde_json::Value,
    max_retries: u32,
    session_id: Option<&str>,
) -> Result<(reqwest::Response, bool), String> {
    let mut attempt: u32 = 0;
    loop {
        let (err, retryable) =
            match post_chat_completion(client, api_url, api_key, body, session_id).await {
                Ok((response, is_anthropic)) => {
                    if response.status().is_success() {
                        return Ok((response, is_anthropic));
                    }
                    let status = response.status();
                    let body_text = response.text().await.unwrap_or_default();
                    let truncated = if body_text.len() > 512 {
                        &body_text[..body_text.floor_char_boundary(512)]
                    } else {
                        &body_text
                    };
                    (
                        format!("LLM API {}: {}", status, truncated),
                        is_retryable_http_status(status),
                    )
                }
                Err(e) => {
                    let retryable = is_retryable_transport_error(&e);
                    (e, retryable)
                }
            };
        if !retryable || attempt >= max_retries {
            return Err(err);
        }
        attempt += 1;
        // Backoff 1s/2s/4s + [0,250ms) jitter
        let base_ms = 1000u64 * (1u64 << (attempt - 1).min(2));
        let jitter = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_millis() as u64)
            .unwrap_or(0))
            % 250;
        let delay = base_ms + jitter;
        tracing::warn!(
            "[llm] 请求失败（{}），{}ms 后重试（第 {}/{} 次）",
            err,
            delay,
            attempt,
            max_retries
        );
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }
}

/// Extracts the error message from a non-streaming response (OpenAI and Anthropic share the {error:{message}} shape).
pub(crate) fn chat_error_message(val: &serde_json::Value) -> Option<String> {
    val.get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .map(String::from)
}

/// Extracts text content from a non-streaming response (OpenAI choices[0].message.content / Anthropic content text blocks).
pub(crate) fn chat_text_content(val: &serde_json::Value, is_anthropic: bool) -> String {
    if is_anthropic {
        crate::plugins::provider::anthropic::anthropic_text_content(val)
    } else {
        val["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string()
    }
}

/// Extracts finish_reason from a non-streaming response (for diagnosing empty responses in title generation).
pub(crate) fn chat_finish_reason(val: &serde_json::Value, is_anthropic: bool) -> String {
    if is_anthropic {
        val.get("stop_reason")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string()
    } else {
        val["choices"][0]["finish_reason"]
            .as_str()
            .unwrap_or("unknown")
            .to_string()
    }
}
