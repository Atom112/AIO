//! 共享 LLM HTTP 客户端与聊天补全辅助 —— 构建于 provider 插件之上。
//!
//! 供 commands/llm（主流式循环）与 services/memory（提取/仲裁/改写）共用；
//! 位于 plugins 层：依赖 provider 插件做 Anthropic 协议适配，无命令层依赖。

/// 流式 HTTP 客户端（LLM 推理专用）。
///
/// - connect_timeout(5s)：快速检测不可达服务器
/// - timeout(600s)：10 分钟硬上限，防止极端慢推理导致无限流
/// - tcp_keepalive(30s)：检测 TCP 层网络分区
/// - http2_keep_alive_interval(30s)：检测 HTTP/2 连接静默断开
///
/// Chunk 级 inactivity 超时（120s）在 `stream_one_round()` 循环中独立处理。
pub(crate) fn streaming_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(600))
        .tcp_keepalive(std::time::Duration::from_secs(30))
        .http2_keep_alive_interval(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// 非流式 HTTP 客户端（summarize / generate_title / fetch_models）。
///
/// 短请求应有短超时，60s 对非流式 LLM 调用绰绰有余。
pub(crate) fn non_streaming_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// 通过 provider 插件系统规范化 chat completions URL。
/// 对于 Ollama（11434 端口 / localhost / ollama 关键字），自动插入 /v1 前缀。
pub(crate) fn normalize_chat_url(api_url: &str) -> String {
    let mgr = crate::plugins::provider::ProviderManager::new();
    let plugin = mgr.for_url(api_url);
    plugin.chat_completions_url(api_url)
}

/// 发送聊天补全请求，自动适配 Anthropic 原生协议（URL / 鉴权 / 请求体转换）。
/// 返回 (response, is_anthropic)，调用方据此解析响应。
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
            .map_err(|e| e.to_string())?;
        Ok((res, true))
    } else {
        let res = client
            .post(&endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        Ok((res, false))
    }
}

/// 判断 HTTP 状态是否值得重试（限流 + 网关/服务端瞬时故障）。
fn is_retryable_http_status(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 429 | 500 | 502 | 503 | 504 | 529)
}

/// 判断传输层错误是否可重试。本包装仅覆盖 POST 发送阶段（尚未发射任何流式
/// 内容），因此这里的连接类错误全部可安全重试。
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

/// 主模型调用的前置阶段重试（POST + HTTP 状态检查，P2.4）。
///
/// 仅覆盖"尚未开始流式发射"的阶段——重试不会导致前端内容重复。流中段错误
/// （120s 不活跃 / chunk 读取失败 / 用户取消）不在此重试，由上层收尾路径兜底。
/// 传输层错误与 408/429/5xx 状态按 1s/2s/4s + 抖动退避重试，上限 max_retries。
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
        // 退避 1s/2s/4s + [0,250ms) 抖动
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

/// 从非流式响应提取错误信息（OpenAI 与 Anthropic 同为 {error:{message}} 形状）。
pub(crate) fn chat_error_message(val: &serde_json::Value) -> Option<String> {
    val.get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
        .map(String::from)
}

/// 从非流式响应提取文本内容（OpenAI choices[0].message.content / Anthropic content 文本块）。
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

/// 提取非流式响应的 finish_reason（用于标题生成空响应诊断）。
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
