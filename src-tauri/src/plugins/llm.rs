//! Shared LLM HTTP client and chat completion helpers - built on top of the provider plugins.
//!
//! Shared by commands/llm (main streaming loop) and services/memory (extract/judge/rewrite);
//! lives in the plugins layer: depends on provider plugins for Anthropic protocol adaptation, with no dependency on the commands layer.

/// Streaming HTTP client (dedicated to LLM inference).
///
/// - connect_timeout(5s): quickly detect unreachable servers
/// - timeout(600s): 10-minute hard cap, preventing extremely slow inference from streaming forever
/// - tcp_keepalive(30s): detect TCP-level network partitions
/// - http2_keep_alive_interval(30s): detect silently dropped HTTP/2 connections
///
/// Chunk-level inactivity timeout (120s) is handled separately in the `stream_one_round()` loop.
pub(crate) fn streaming_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(600))
        .tcp_keepalive(std::time::Duration::from_secs(30))
        .http2_keep_alive_interval(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Non-streaming HTTP client (summarize / generate_title / fetch_models).
///
/// Short requests should have short timeouts; 60s is ample for non-streaming LLM calls.
pub(crate) fn non_streaming_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Normalizes the chat completions URL via the provider plugin system.
/// For Ollama (port 11434 / localhost / ollama keyword), the /v1 prefix is inserted automatically.
pub(crate) fn normalize_chat_url(api_url: &str) -> String {
    let mgr = crate::plugins::provider::ProviderManager::new();
    let plugin = mgr.for_url(api_url);
    plugin.chat_completions_url(api_url)
}

/// Sends a chat completion request, auto-adapting to the native Anthropic protocol (URL / auth / request body conversion).
/// Returns (response, is_anthropic); the caller parses the response accordingly.
pub(crate) async fn post_chat_completion(
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    body: &serde_json::Value,
) -> Result<(reqwest::Response, bool), String> {
    let endpoint = normalize_chat_url(api_url);
    let is_anthropic = crate::plugins::provider::anthropic::is_anthropic_url(api_url);
    if is_anthropic {
        let anthropic_body = crate::plugins::provider::anthropic::to_anthropic_body(body)?;
        let res = client
            .post(&endpoint)
            .header("x-api-key", api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&anthropic_body)
            .send()
            .await
            // PROV-11: reqwest's Display carries the full URL (Gemini-style auth puts
            // the key in ?key=...) — redact before the text reaches toasts/logs.
            // The "error sending request …" prefix is preserved for retry detection.
            .map_err(|e| crate::plugins::provider::redact_urls_in(&e.to_string()))?;
        Ok((res, true))
    } else {
        let res = client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(body)
            .send()
            .await
            .map_err(|e| crate::plugins::provider::redact_urls_in(&e.to_string()))?;
        Ok((res, false))
    }
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
) -> Result<(reqwest::Response, bool), String> {
    let mut attempt: u32 = 0;
    loop {
        let (err, retryable) = match post_chat_completion(client, api_url, api_key, body).await {
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
