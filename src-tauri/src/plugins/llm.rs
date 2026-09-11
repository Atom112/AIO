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
