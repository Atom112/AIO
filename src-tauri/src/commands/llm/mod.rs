pub mod btw;

use crate::commands::attachment::sync_message_attachments;
use crate::core::models::*;
use crate::core::permission::{self, PermissionAction};
use crate::core::state::DbState;
use crate::core::state::StreamManager;
use crate::core::subagent;
use crate::plugins::mcp::PendingApprovals;
use crate::utils::file_tools;
use crate::utils::git_tools;
use crate::utils::knowledge;
use crate::utils::lsp_agent_tools;
use crate::utils::lsp_tools;
use crate::utils::shell_tools;
use crate::utils::web_tools;
use base64::{engine::general_purpose, Engine as _};
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt; // 用于处理流式数据
use rusqlite::params;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Window}; // Emitter 用于从后端向前端推送事件
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

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

/// 单轮流式调用后的累积结果。
struct RoundResult {
    /// 本轮 assistant 文本内容（含重写后的 aio-image 标记）
    content: String,
    /// 本轮思维链内容（实时已通过 llm-reasoning 事件下发，此处仅留档供调试）
    #[allow(dead_code)]
    reasoning: String,
    /// 本轮模型发起的工具调用（按 index 升序）
    tool_calls: Vec<ToolCallAccum>,
    /// 服务端返回的 prompt tokens（输入用量）
    input_tokens: u32,
    /// 服务端返回的 completion tokens（输出用量）
    output_tokens: u32,
    /// 缓存命中的输入 tokens（OpenAI cached_tokens / DeepSeek prompt_cache_hit_tokens）
    cached_input_tokens: u32,
    /// 本轮生成/落盘的图像元数据
    images: Vec<GeneratedImage>,
    /// 回送 API / 持久化的原始载荷形态（未重写 token 的数组或字符串），用于下一轮请求
    api_content: serde_json::Value,
    /// 服务端返回的 finish_reason（如 "stop" / "tool_calls" / "length"），用于截断恢复
    finish_reason: Option<String>,
}

/// 累积完成的单个工具调用。
#[derive(Clone)]
struct ToolCallAccum {
    id: String,
    name: String,
    arguments: String,
}

/// 流式 tool_call 累积载荷（发往前端用，仅用于通知前端展示"调用中"气泡）
#[derive(Serialize, Clone)]
pub struct ToolCallPayload {
    pub assistant_id: String,
    pub topic_id: String,
    pub tool_call_id: String,
    pub name: String,
    pub arguments: String,
}

fn message_for_api(
    conn: &rusqlite::Connection,
    message: &Message,
) -> Result<serde_json::Value, String> {
    let mut content = message.content.clone();
    if let Some(files) = &message.display_files {
        if files.iter().any(|file| file.id.is_some()) {
            let base_text = match &message.content {
                serde_json::Value::String(text) => text.clone(),
                other => extract_text_content(other),
            };
            let mut document_sections = Vec::new();
            let mut image_data_urls = Vec::new();

            for file in files {
                let Some(attachment_id) = file.id.as_deref() else {
                    continue;
                };
                let attachment = conn
                    .query_row(
                        "SELECT file_name, mime_type, storage_path, extracted_text
                         FROM attachments WHERE id = ?1",
                        [attachment_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, Option<String>>(3)?,
                            ))
                        },
                    )
                    .map_err(|e| format!("读取附件 {} 失败: {}", file.name, e))?;

                if attachment.1.starts_with("image/") {
                    let bytes = std::fs::read(&attachment.2)
                        .map_err(|e| format!("读取图片附件 {} 失败: {}", attachment.0, e))?;
                    image_data_urls.push(format!(
                        "data:{};base64,{}",
                        attachment.1,
                        general_purpose::STANDARD.encode(bytes)
                    ));
                } else if let Some(text) = attachment.3 {
                    document_sections.push(format!("[{}]\n{}", file.name, text));
                }
            }

            let expanded_text = if document_sections.is_empty() {
                base_text
            } else {
                format!(
                    "参考文件内容：\n{}\n---\n{}",
                    document_sections.join("\n"),
                    base_text
                )
            };
            content = if image_data_urls.is_empty() {
                json!(expanded_text)
            } else {
                let mut parts = vec![json!({ "type": "text", "text": expanded_text })];
                parts.extend(
                    image_data_urls
                        .into_iter()
                        .map(|url| json!({ "type": "image_url", "image_url": { "url": url } })),
                );
                serde_json::Value::Array(parts)
            };
        }
    }

    // 模型生成图像：剥离 aio-image 标记并把 images 元数据展开回 image_url 块（历史回放）
    if let Some(images) = &message.images {
        if !images.is_empty() {
            let base_text = match &content {
                serde_json::Value::String(text) => {
                    crate::utils::generated_images::strip_generated_image_tokens(text)
                }
                other => extract_text_content(other),
            };
            let mut parts = vec![json!({ "type": "text", "text": base_text })];
            for img in images {
                if let Ok(bytes) = std::fs::read(&img.storage_path) {
                    parts.push(json!({
                        "type": "image_url",
                        "image_url": {
                            "url": format!(
                                "data:{};base64,{}",
                                img.mime_type,
                                general_purpose::STANDARD.encode(bytes)
                            )
                        }
                    }));
                } else {
                    tracing::warn!("读取生成图片失败: {}", img.storage_path);
                }
            }
            content = serde_json::Value::Array(parts);
        }
    }

    let mut object = serde_json::Map::new();
    object.insert("role".into(), json!(message.role));
    object.insert("content".into(), content);
    if let Some(tool_call_id) = &message.tool_call_id {
        object.insert("tool_call_id".into(), json!(tool_call_id));
    }
    if let Some(name) = &message.name {
        object.insert("name".into(), json!(name));
    }
    if let Some(tool_calls) = &message.tool_calls {
        object.insert("tool_calls".into(), json!(tool_calls));
    }
    Ok(serde_json::Value::Object(object))
}

/// 防御性校验：扫描 messages 中的 assistant(tool_calls) 与 role:tool 的匹配情况，
/// 发现缺失或不匹配时输出 warning 日志以便调试。
fn verify_tool_messages(messages: &[serde_json::Value]) {
    // 从前往后扫描，追踪每个 assistant 消息中声明的 tool_call_id
    let mut pending_ids: Vec<String> = Vec::new();
    let mut assistant_idx: Option<usize> = None;

    for (i, msg) in messages.iter().enumerate() {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "assistant" {
            if let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                if !tcs.is_empty() {
                    // 新的 assistant(tool_calls) 开始，之前的 pending_ids 尚未匹配 → 缺失
                    if !pending_ids.is_empty() {
                        tracing::warn!(
                            "[verify_tool_messages] assistant[{}] 的 tool_calls {:?} 缺少对应 tool 响应",
                            assistant_idx.unwrap_or(0),
                            pending_ids
                        );
                    }
                    pending_ids = tcs
                        .iter()
                        .filter_map(|tc| tc.get("id").and_then(|v| v.as_str()).map(String::from))
                        .collect();
                    assistant_idx = Some(i);
                }
            }
        } else if role == "tool" {
            if let Some(tool_call_id) = msg.get("tool_call_id").and_then(|v| v.as_str()) {
                if let Some(pos) = pending_ids.iter().position(|id| id == tool_call_id) {
                    pending_ids.remove(pos);
                } else {
                    tracing::warn!(
                        "[verify_tool_messages] tool[{}] 的 tool_call_id `{}` 在前一条 assistant 中未找到对应的 tool_call",
                        i,
                        tool_call_id
                    );
                }
            }
        }
    }

    // 扫描结束，仍有未匹配的 tool_call_id
    if !pending_ids.is_empty() {
        tracing::warn!(
            "[verify_tool_messages] 扫描结束: assistant[{}] 的 tool_calls {:?} 缺少对应 tool 响应",
            assistant_idx.unwrap_or(0),
            pending_ids
        );
    }
}

/// 将单轮 LLM 调用的 token 用量写入 usage_log 表（不可变 append-only 记录）。
///
/// 跳过 input + output 均为 0 的空记录（本地引擎可能不返回 usage）。
#[allow(clippy::too_many_arguments)]
fn insert_usage_log(
    app: &AppHandle,
    assistant_id: &str,
    topic_id: &str,
    model_id: &str,
    round: u32,
    input_tokens: u32,
    output_tokens: u32,
    cached_input_tokens: u32,
) {
    if input_tokens == 0 && output_tokens == 0 {
        return;
    }
    let db = app.state::<DbState>();
    let conn = db.0.lock();
    let id = uuid::Uuid::new_v4().to_string();
    let _ = conn.execute(
        "INSERT INTO usage_log (id, assistant_id, topic_id, model_id, round, input_tokens, output_tokens, cached_input_tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            id,
            assistant_id,
            topic_id,
            model_id,
            round,
            input_tokens,
            output_tokens,
            cached_input_tokens
        ],
    );
}

/// 通过 provider 插件系统规范化 chat completions URL。
/// 对于 Ollama（11434 端口 / localhost / ollama 关键字），自动插入 /v1 前缀。
fn normalize_chat_url(api_url: &str) -> String {
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
fn chat_finish_reason(val: &serde_json::Value, is_anthropic: bool) -> String {
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

/// 单轮流式请求：构造 body → POST → 解析 SSE → 累积 content/reasoning/tool_calls → emit 增量事件。
///
/// 与旧 `call_llm_stream` 的差异：
/// - 累积器用 `BTreeMap`（按 index 升序），避免多工具乱序；
/// - 用 `tokio::select!` 监听 `token.cancelled()`，取消时立即返回 `Err("cancelled")`，
///   保证调用方能继续执行 epilogue；
/// - **不** emit terminal `llm-chunk(done)`，由调用方（`run_agent_turn`）统一收尾；
/// - tool_call 仅 emit `llm-tool-call`（通知前端展示"调用中"气泡），执行由循环主体负责。
///
/// 返回 `RoundResult`；若被取消返回 `Err("cancelled")`，其它错误原样上抛。
#[allow(
    clippy::too_many_arguments,
    reason = "stream orchestration keeps request and event context explicit"
)]
async fn stream_one_round(
    window: &Window,
    token: &CancellationToken,
    client: &reqwest::Client,
    api_url: String,
    api_key: &str,
    model: &str,
    messages: &[serde_json::Value],
    tools: Option<&[ToolSpec]>,
    assistant_id: &str,
    topic_id: &str,
    suppress_events: bool,
    app: Option<&AppHandle>,
    process_images: bool,
    max_output_tokens: Option<u32>,
) -> Result<RoundResult, String> {
    let mut body_map = serde_json::Map::new();
    body_map.insert("model".into(), json!(model));
    body_map.insert("messages".into(), json!(messages));
    body_map.insert("stream".into(), json!(true));
    if let Some(tools) = tools {
        if !tools.is_empty() {
            body_map.insert("tools".into(), json!(tools));
            body_map.insert("tool_choice".into(), json!("auto"));
        }
    }
    // 可选的输出长度上限：防止单轮输出无界膨胀（推理类模型 o1/o3/gpt-5 系列需用
    // max_completion_tokens，这里由调用方负责按模型类型决定是否传入）
    if let Some(mt) = max_output_tokens {
        body_map.insert("max_tokens".into(), json!(mt));
    }
    let body = serde_json::Value::Object(body_map);

    // 统一请求入口：Anthropic 原生协议（/v1/messages + x-api-key + body 转换）自动适配
    let (response, is_anthropic_request) =
        post_chat_completion(client, &api_url, api_key, &body).await?;

    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        let truncated = if body_text.len() > 512 {
            &body_text[..body_text.floor_char_boundary(512)]
        } else {
            &body_text
        };
        return Err(format!("LLM API {}: {}", status, truncated));
    }

    let mut stream = response.bytes_stream();
    let mut line_buffer = String::new();
    // tool_call 累积：index → (id, name, arguments)，用 BTreeMap 保证按 index 升序 flush
    let mut tc_accum: BTreeMap<usize, (String, String, String)> = BTreeMap::new();

    let mut content_buf = String::new();
    let mut reasoning_buf = String::new();
    // 服务端返回的 finish_reason（"stop" / "tool_calls" / "length"），供调用方做截断恢复
    let mut finish_reason: Option<String> = None;
    // 流式数组 delta 里解析出的 image_url 与原始 API 载荷（回送下一轮时用未重写形态，防 token 泄漏）
    let mut image_urls: Vec<String> = Vec::new();
    let mut round_api_parts: Vec<serde_json::Value> = Vec::new();
    let mut saw_done = false;
    // 从 SSE 流末尾提取 token 用量（服务端返回）
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;
    let mut cached_input_tokens: u32 = 0;

    loop {
        // 三重竞争：取消信号 / stream chunk / 120s inactivity 超时
        let next = tokio::select! {
            _ = token.cancelled() => return Err("cancelled".to_string()),
            item = stream.next() => item,
            _ = tokio::time::sleep(std::time::Duration::from_secs(120)) => {
                return Err("LLM 流超时：120 秒未收到数据，连接可能已断开".to_string());
            }
        };

        let chunk = match next {
            Some(Ok(c)) => c,
            Some(Err(e)) => return Err(e.to_string()),
            None => break, // 流自然结束
        };
        line_buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = line_buffer.find('\n') {
            let line = line_buffer[..pos].trim().to_string();
            line_buffer.drain(..pos + 1);

            if line.is_empty() {
                continue;
            }

            if line == "data: [DONE]" {
                saw_done = true;
                break;
            }

            if let Some(json_str) = line.strip_prefix("data: ") {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if is_anthropic_request {
                        // Anthropic Messages 流式事件：内容/思考/工具调用/用量统一累积
                        crate::plugins::provider::anthropic::handle_anthropic_stream_event(
                            &val,
                            window,
                            assistant_id,
                            topic_id,
                            &mut content_buf,
                            &mut reasoning_buf,
                            &mut tc_accum,
                            &mut finish_reason,
                            &mut input_tokens,
                            &mut output_tokens,
                            &mut cached_input_tokens,
                        );
                        if val.get("type").and_then(|v| v.as_str()) == Some("message_stop") {
                            saw_done = true;
                            break;
                        }
                    } else if let Some(content) = val["choices"][0]["delta"]["content"].as_str() {
                        content_buf.push_str(content);
                        if !suppress_events {
                            let _ = window.emit(
                                "llm-chunk",
                                StreamPayload {
                                    assistant_id: assistant_id.to_string(),
                                    topic_id: topic_id.to_string(),
                                    content: content.to_string(),
                                    done: false,
                                    error: None,
                                    input_tokens: None,
                                    output_tokens: None,
                                    context_tokens: None,
                                    cached_input_tokens: None,
                                    images: None,
                                },
                            );
                        }
                    } else if let Some(parts) = val["choices"][0]["delta"]["content"].as_array() {
                        // 图像类模型（vLLM Qwen-Image、OpenAI gpt-image-1 chat 模式等）以数组
                        // 交付文本与图片块。文本逐块推送；图片块收集 URL，待流结束后落盘重写。
                        for part in parts {
                            match part.get("type").and_then(|v| v.as_str()) {
                                Some("text") => {
                                    if let Some(s) = part.get("text").and_then(|v| v.as_str()) {
                                        content_buf.push_str(s);
                                        if !suppress_events {
                                            let _ = window.emit(
                                                "llm-chunk",
                                                StreamPayload {
                                                    assistant_id: assistant_id.to_string(),
                                                    topic_id: topic_id.to_string(),
                                                    content: s.to_string(),
                                                    done: false,
                                                    error: None,
                                                    input_tokens: None,
                                                    output_tokens: None,
                                                    context_tokens: None,
                                                    cached_input_tokens: None,
                                                    images: None,
                                                },
                                            );
                                        }
                                        round_api_parts.push(json!({ "type": "text", "text": s }));
                                    }
                                }
                                Some("image_url") | Some("output_image") => {
                                    let url = part
                                        .get("image_url")
                                        .and_then(|v| v.get("url"))
                                        .and_then(|v| v.as_str())
                                        .or_else(|| part.get("url").and_then(|v| v.as_str()));
                                    if let Some(u) = url {
                                        image_urls.push(u.to_string());
                                        round_api_parts.push(json!({
                                            "type": "image_url",
                                            "image_url": { "url": u }
                                        }));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    if let Some(reasoning) = val["choices"][0]["delta"]["reasoning_content"]
                        .as_str()
                        .or_else(|| val["choices"][0]["delta"]["reasoning"].as_str())
                    {
                        if !reasoning.is_empty() {
                            reasoning_buf.push_str(reasoning);
                            if !suppress_events {
                                let _ = window.emit(
                                    "llm-reasoning",
                                    StreamPayload {
                                        assistant_id: assistant_id.to_string(),
                                        topic_id: topic_id.to_string(),
                                        content: reasoning.to_string(),
                                        done: false,
                                        error: None,
                                        input_tokens: None,
                                        output_tokens: None,
                                        context_tokens: None,
                                        cached_input_tokens: None,
                                        images: None,
                                    },
                                );
                            }
                        }
                    }
                    // tool_calls 累积
                    if let Some(tcs) = val["choices"][0]["delta"]["tool_calls"].as_array() {
                        for tc in tcs {
                            let index =
                                tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                            let entry = tc_accum
                                .entry(index)
                                .or_insert_with(|| (String::new(), String::new(), String::new()));
                            if let Some(id) = tc.get("id").and_then(|v| v.as_str()) {
                                entry.0 = id.to_string();
                            }
                            if let Some(name) = tc
                                .get("function")
                                .and_then(|f| f.get("name"))
                                .and_then(|v| v.as_str())
                            {
                                entry.1 = name.to_string();
                            }
                            if let Some(args) = tc
                                .get("function")
                                .and_then(|f| f.get("arguments"))
                                .and_then(|v| v.as_str())
                            {
                                entry.2.push_str(args);
                            }
                        }
                    }
                    // finish_reason="tool_calls" 时不必立即 flush（累积器已存好），
                    // 统一在末尾按 index 升序构造。这里仅触发提前 flush 事件通知前端。
                    let finish = val["choices"][0]["finish_reason"].as_str().unwrap_or("");
                    if !finish.is_empty() {
                        finish_reason = Some(finish.to_string());
                    }
                    if finish == "tool_calls" && !suppress_events {
                        for (id, name, args) in tc_accum.values() {
                            if !id.is_empty() && !name.is_empty() {
                                let _ = window.emit(
                                    "llm-tool-call",
                                    ToolCallPayload {
                                        assistant_id: assistant_id.to_string(),
                                        topic_id: topic_id.to_string(),
                                        tool_call_id: id.clone(),
                                        name: name.clone(),
                                        arguments: args.clone(),
                                    },
                                );
                            }
                        }
                    }
                    // 提取服务端返回的 token 用量（OpenAI 兼容 API 在最后一个 chunk 中附带 usage）
                    if let Some(usage) = val.get("usage") {
                        if let Some(pt) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                            input_tokens = pt as u32;
                        }
                        if let Some(ct) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                            output_tokens = ct as u32;
                        }
                        // 缓存命中统计：OpenAI 在 prompt_tokens_details.cached_tokens，DeepSeek 在 prompt_cache_hit_tokens
                        if let Some(cached) = usage
                            .get("prompt_tokens_details")
                            .and_then(|d| d.get("cached_tokens"))
                            .and_then(|v| v.as_u64())
                            .or_else(|| {
                                usage
                                    .get("prompt_cache_hit_tokens")
                                    .and_then(|v| v.as_u64())
                            })
                        {
                            cached_input_tokens = cached as u32;
                        }
                    }
                } // else 分支结束（OpenAI 兼容解析）
            }
        }
        if saw_done {
            break;
        }
    }

    // 按 index 升序构造工具调用列表（若 finish_reason="tool_calls" 已 emit 过通知，这里不再重复 emit）
    let mut tool_calls = Vec::new();
    let need_emit = !saw_done; // 若未到 [DONE]，则此前可能未 emit tool-call 通知
    for (id, name, args) in tc_accum.values() {
        if !id.is_empty() && !name.is_empty() {
            tool_calls.push(ToolCallAccum {
                id: id.clone(),
                name: name.clone(),
                arguments: args.clone(),
            });
        }
    }
    // 若 finish_reason="tool_calls" 路径未触发（某些 provider 只在末尾给 tool_calls delta），
    // 这里补发 llm-tool-call 通知，确保前端展示。
    if need_emit && !suppress_events {
        for tc in &tool_calls {
            let _ = window.emit(
                "llm-tool-call",
                ToolCallPayload {
                    assistant_id: assistant_id.to_string(),
                    topic_id: topic_id.to_string(),
                    tool_call_id: tc.id.clone(),
                    name: tc.name.clone(),
                    arguments: tc.arguments.clone(),
                },
            );
        }
    }

    // 回送 API 的载荷形态：有数组块时用原始数组（未重写 token），否则退回纯文本字符串。
    let api_content = if round_api_parts.is_empty() {
        json!(content_buf)
    } else {
        json!(round_api_parts)
    };
    let (final_content, images) = if process_images && !image_urls.is_empty() {
        let mut c = content_buf.clone();
        match crate::utils::generated_images::save_generated_images(
            app.unwrap(),
            &mut c,
            &image_urls,
        )
        .await
        {
            Ok(imgs) => (c, imgs),
            Err(e) => {
                tracing::warn!("保存生成图片失败: {}", e);
                (content_buf.clone(), vec![])
            }
        }
    } else {
        (content_buf.clone(), vec![])
    };

    Ok(RoundResult {
        content: final_content,
        reasoning: reasoning_buf,
        tool_calls,
        input_tokens,
        output_tokens,
        cached_input_tokens,
        images,
        api_content,
        finish_reason,
    })
}

/// 核心函数：调用 LLM 并分块回传结果（流式输出）。
///
/// 注意：此命令保留用于命令稳定性，但前端新流程改用 [`run_agent_turn`]
/// （后端单任务自驱循环）。本命令现在仅做单轮流式 + 终止 done，
/// 不再做工具执行/递归——工具调用的通知事件仍会 emit（供调试/兼容）。
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri IPC compatibility requires the existing flat command parameters"
)]
pub async fn call_llm_stream(
    window: Window,                         // Tauri 窗口句柄，用于发送事件
    state: tauri::State<'_, StreamManager>, // 全局状态，用于管理正在进行的流任务
    db_state: tauri::State<'_, DbState>,
    api_url: String,              // API 地址
    api_key: String,              // API 密钥
    model: String,                // 模型名称（如 gpt-3.5-turbo）
    assistant_id: String,         // 助手 ID（用于前端匹配消息）
    topic_id: String,             // 话题/会话 ID
    messages: Vec<Message>,       // 历史上下文消息列表
    tools: Option<Vec<ToolSpec>>, // 工具定义（MCP 工具，None 或空数组则不发送）
) -> Result<(), String> {
    // 诊断日志：打印 messages 每个元素的 role 和关键字段是否存在
    for (i, m) in messages.iter().enumerate() {
        tracing::info!(
            "[call_llm_stream] messages[{}] role={} has_toolCallId={} has_toolCalls={} has_name={}",
            i,
            m.role,
            m.tool_call_id.is_some(),
            m.tool_calls.is_some(),
            m.name.is_some()
        );
    }
    // SSRF 防护
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    // 1. 生成唯一的任务 Key，格式为 "助手ID-话题ID"
    let task_key = format!("{}-{}", assistant_id, topic_id);

    // 2. 如果当前 Key 已有任务在运行，先取消旧任务（cancel 而非 abort，保证 epilogue）
    if let Some((_, (_, old_token))) = state.0.remove(&task_key) {
        old_token.cancel();
    }

    // 3. 克隆变量以便进入异步线程（move 闭包）
    let state_inner = state.0.clone();
    let task_key_inner = task_key.clone();
    let assistant_id_c = assistant_id.clone();
    let topic_id_c = topic_id.clone();
    let messages_for_api = {
        let conn = db_state.0.lock();
        messages
            .iter()
            .map(|message| message_for_api(&conn, message))
            .collect::<Result<Vec<_>, _>>()?
    };
    let tools_slice = tools; // 用于 as_slice()

    // 防御性校验：检查 tool_calls 与 tool 响应是否匹配
    verify_tool_messages(&messages_for_api);

    let token = CancellationToken::new();
    let token_inner = token.clone();
    let app_handle = window.app_handle().clone();
    let model_c = model.clone();

    // 4. 创建异步任务执行请求
    let handle = tokio::spawn(async move {
        let client = streaming_http_client();
        let tools_ref: Option<&[ToolSpec]> = tools_slice.as_deref();
        let result = stream_one_round(
            &window,
            &token_inner,
            &client,
            api_url,
            &api_key,
            &model,
            &messages_for_api,
            tools_ref,
            &assistant_id_c,
            &topic_id_c,
            false,
            Some(&app_handle),
            true,
            None,
        )
        .await;

        // 收尾：无论成功/取消/错误都 emit terminal done，保证前端 isThinking 必复位
        match result {
            Ok(round) => {
                // 持久化本轮 token 用量到 usage_log
                insert_usage_log(
                    &app_handle,
                    &assistant_id_c,
                    &topic_id_c,
                    &model_c,
                    1,
                    round.input_tokens,
                    round.output_tokens,
                    round.cached_input_tokens,
                );
                let _ = window.emit(
                    "llm-chunk",
                    StreamPayload {
                        assistant_id: assistant_id_c.clone(),
                        topic_id: topic_id_c.clone(),
                        content: if round.images.is_empty() {
                            "".into()
                        } else {
                            round.content.clone()
                        },
                        done: true,
                        error: None,
                        input_tokens: Some(round.input_tokens),
                        output_tokens: Some(round.output_tokens),
                        context_tokens: Some(round.input_tokens),
                        cached_input_tokens: Some(round.cached_input_tokens),
                        images: if round.images.is_empty() {
                            None
                        } else {
                            Some(round.images.clone())
                        },
                    },
                );
            }
            Err(e) => {
                tracing::error!("Stream Error: {}", e);
                let is_cancel = e == "cancelled";
                let _ = window.emit(
                    "llm-chunk",
                    StreamPayload {
                        assistant_id: assistant_id_c.clone(),
                        topic_id: topic_id_c.clone(),
                        content: if is_cancel {
                            "".into()
                        } else {
                            format!("\n[Error: {}]", e)
                        },
                        done: true,
                        error: if is_cancel { None } else { Some(e) },
                        input_tokens: None,
                        output_tokens: None,
                        context_tokens: None,
                        cached_input_tokens: None,
                        images: None,
                    },
                );
            }
        }

        // 任务完成后，从全局状态中移除
        state_inner.remove(&task_key_inner);
    });

    // 5. 将当前正在执行的任务句柄与取消令牌存入全局状态
    state.0.insert(task_key, (handle, token));
    Ok(())
}

/// 辅助函数：从服务商获取可用的模型列表
#[tauri::command]
pub async fn fetch_models(api_url: String, api_key: String) -> Result<Vec<ModelInfo>, String> {
    // SSRF 防护
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    // 构造模型获取地址，通常是基础 URL 后接 /models
    let mut base_url = api_url.trim_end_matches('/').to_string();
    if base_url.ends_with("/chat/completions") {
        base_url = base_url.replace("/chat/completions", "");
    }
    let final_url = format!("{}/models", base_url);

    let client = non_streaming_http_client();
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        client
            .get(&final_url)
            .header("Authorization", format!("Bearer {}", api_key))
            .send(),
    )
    .await
    .map_err(|_| "获取模型列表超时（45s）".to_string())?
    .map_err(|e| e.to_string())?;

    // 解析返回的模型 JSON 数据
    let res_data: ModelsResponse = response.json().await.map_err(|e| e.to_string())?;
    Ok(res_data.data)
}

/// 停止函数：用户点击“停止生成”时调用。
///
/// 通过 `CancellationToken::cancel()` 通知任务优雅退出（而非 `abort()`），
/// 任务在 `select!` 分支返回后仍能执行 epilogue（emit done + 移除自身），
/// 保证前端的 `isThinking` 状态必然被复位。
#[tauri::command]
pub async fn stop_llm_stream(
    state: tauri::State<'_, StreamManager>,
    assistant_id: String,
    topic_id: String,
) -> Result<(), String> {
    let task_key = format!("{}-{}", assistant_id, topic_id);

    // 取出取消令牌并触发 cancel；任务自身负责 emit done 与从状态中移除。
    // 注意：这里只 cancel，不 abort，保证 epilogue 必达。
    if let Some(entry) = state.0.get(&task_key) {
        entry.1.cancel();
    }
    Ok(())
}

/// 执行内置工具（in-process 直接调用，含权限检查和审批）。
///
/// 覆盖文件工具（file_tools）和命令执行工具（shell_tools），跳过 MCP 通道。
/// 权限模型与 `execute_tool_call` 保持一致。
async fn execute_builtin_tool(
    app: &AppHandle,
    tool_name: &str,
    arguments: &serde_json::Value,
    project_id: Option<&str>,
    agent_mode: &AgentMode,
    token: &CancellationToken,
) -> Result<ToolResult, String> {
    let project_root = file_tools::resolve_project_root(app, project_id)?;

    // 权限检查（与 execute_tool_call 保持一致，server_id 沿用 "__aio-filesystem__" 以兼容已有规则）
    if *agent_mode != AgentMode::Off {
        let custom_rules = permission::load_permissions(Some(&project_root)).rules;
        let action = permission::check_permission(
            tool_name,
            "__aio-filesystem__",
            arguments,
            agent_mode,
            &custom_rules,
        );

        match action {
            PermissionAction::Deny => {
                return Err(format!(
                    "工具 '{}' 已在当前模式下被安全策略禁止执行（Deny）。\n如需执行，请切换到自动模式或在项目权限设置中添加 allow 规则。",
                    tool_name
                ));
            }
            PermissionAction::Ask => {
                let pending = app.state::<PendingApprovals>();
                let mut reason = format!("工具 '{}' 需要您的确认才能执行", tool_name);
                // 命令执行：危险命令标注
                if tool_name == "execute_command" {
                    if let Some(cmd) = arguments["command"].as_str() {
                        if let Some(danger) = shell_tools::check_dangerous_command(cmd) {
                            let prefix = match danger.category {
                                shell_tools::DangerCategory::Critical => "🚨 严重危险",
                                shell_tools::DangerCategory::High => "⚠️ 高风险",
                                shell_tools::DangerCategory::Medium => "⚡ 中等风险",
                            };
                            reason = format!(
                                "{prefix}\n\n{danger_desc}\n\n命令: {cmd}\n\n确认执行？",
                                prefix = prefix,
                                danger_desc = danger.risk,
                            );
                        } else {
                            reason =
                                format!("命令: {cmd}\n\n工具 '{}' 需要您的确认才能执行", tool_name);
                        }
                    }
                }
                // 文件修改工具：预计算 diff 预览
                let (preview_diff, file_path) =
                    compute_diff_preview(tool_name, arguments, &project_root);
                let approval_fut = crate::commands::mcp::request_tool_approval(
                    app,
                    pending.inner(),
                    "__aio-filesystem__",
                    tool_name,
                    arguments,
                    &reason,
                    preview_diff,
                    file_path,
                );
                tokio::select! {
                    _ = token.cancelled() => return Err("cancelled".into()),
                    res = approval_fut => res?,
                }
            }
            PermissionAction::Allow => {}
        }
    }

    // 分发执行
    if tool_name == "execute_command" {
        let command = arguments["command"].as_str().unwrap_or("").to_string();
        let timeout = arguments["timeout"].as_u64();
        let root = project_root.clone();
        Ok(tokio::task::spawn_blocking(move || {
            shell_tools::execute_command(&command, &root, timeout)
        })
        .await
        .unwrap_or_else(|_| shell_tools::tool_err("命令执行线程 panic")))
    } else if tool_name == "web_fetch" {
        let url = arguments["url"].as_str().unwrap_or("");
        let max_bytes = arguments["max_bytes"].as_u64();
        Ok(web_tools::execute_web_fetch(url, max_bytes).await)
    } else if tool_name == "web_search" {
        let query = arguments["query"].as_str().unwrap_or("");
        let count = arguments["count"].as_u64();
        Ok(web_tools::execute_web_search(query, count).await)
    } else if tool_name == "read_lints" {
        lsp_tools::execute(app, &project_root, arguments).await
    } else if tool_name == "lsp_definition" {
        lsp_agent_tools::execute_lsp_definition(app, &project_root, arguments).await
    } else if tool_name == "lsp_references" {
        lsp_agent_tools::execute_lsp_references(app, &project_root, arguments).await
    } else if tool_name == "lsp_hover" {
        lsp_agent_tools::execute_lsp_hover(app, &project_root, arguments).await
    } else if tool_name == "lsp_symbols" {
        lsp_agent_tools::execute_lsp_symbols(app, &project_root, arguments).await
    } else if matches!(
        tool_name,
        "remember" | "recall" | "search_memory" | "update_memory" | "forget_memory"
    ) {
        let tool_name_c = tool_name.to_string();
        let arguments_c = arguments.clone();
        let root_c = project_root.clone();
        let app_c2 = app.clone();
        crate::services::memory::tools::execute_memory_tool(
            &app_c2,
            &tool_name_c,
            &arguments_c,
            &root_c,
        )
        .await
    } else if tool_name == "think" {
        Ok(crate::utils::think::execute(arguments))
    } else if tool_name == "project_map" {
        Ok(crate::utils::project_map::execute(&project_root))
    } else if tool_name.starts_with("git_") {
        let tool_name_c = tool_name.to_string();
        let arguments_c = arguments.clone();
        let root_c = project_root.clone();
        Ok(tokio::task::spawn_blocking(move || {
            git_tools::execute_git_tool(&tool_name_c, &arguments_c, &root_c)
        })
        .await
        .unwrap_or_else(|_| git_tools::tool_err("Git 工具执行线程 panic")))
    } else {
        let tool_name_c = tool_name.to_string();
        let arguments_c = arguments.clone();
        let root_c = project_root.clone();
        Ok(tokio::task::spawn_blocking(move || {
            file_tools::execute_file_tool(&tool_name_c, &arguments_c, &root_c)
        })
        .await
        .unwrap_or_else(|_| file_tools::tool_err("文件工具执行线程 panic")))
    }
}

/// 截断文本用于显示（保留前 max_len 字符并追加 "…"）
fn truncate_for_display(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max_len).collect::<String>())
    }
}

/// 构建子 Agent 的消息列表（系统提示词 + 任务描述 + 上下文提示）
fn build_subagent_messages(
    profile: &subagent::SubagentProfile,
    task_desc: &str,
    context_files: &[String],
    project_root: &str,
    extra_context: Option<&str>,
    locale: &str,
) -> Vec<serde_json::Value> {
    let system_prompt = if locale == "en-US" {
        format!(
            "You are an AIO subagent with the role \"{}\".\nWorking directory: {}\nComplete the task efficiently and summarize the result at the end.\n\n{}\n\nImportant: end with a concise work summary including changed file paths and key findings.",
            profile.name, project_root, profile.system_prompt_extension,
        )
    } else {
        format!(
            "你是 AIO 的子智能体，类型为「{}」。\n工作目录: {}\n请高效完成子任务并在最后总结工作成果。\n\n{}\n\n重要：请在回复末尾给出简洁的工作总结（含修改的文件路径和关键发现）。回复请保持精炼，重点突出核心成果，避免冗长叙述。",
            profile.name, project_root, profile.system_prompt_extension,
        )
    };
    let task_prompt = if locale == "en-US" {
        format!(
            "Subtask:\n{}\n\nBegin now and finish with a work summary.",
            task_desc
        )
    } else {
        format!(
            "子任务:\n{}\n\n请开始执行。完成后在最后给出工作总结。",
            task_desc
        )
    };

    let mut msgs: Vec<serde_json::Value> = vec![
        json!({"role": "system", "content": system_prompt}),
        json!({"role": "user", "content": task_prompt}),
    ];

    if !context_files.is_empty() {
        let files_hint = context_files
            .iter()
            .map(|f| format!("- {}", f))
            .collect::<Vec<_>>()
            .join("\n");
        msgs.push(json!({
            "role": "user",
            "content": if locale == "en-US" {
                format!("The following files may be relevant. Read them as needed:\n{}", files_hint)
            } else {
                format!("提示：以下文件可能与任务相关，请根据需要读取：\n{}", files_hint)
            }
        }));
    }

    if let Some(ctx) = extra_context {
        if !ctx.is_empty() {
            msgs.push(json!({
                "role": "user",
                "content": format!("{}\n{}", if locale == "en-US" { "[Shared context]" } else { "[共享上下文]" }, ctx)
            }));
        }
    }

    msgs
}

/// 构建子 Agent 的受限工具列表（根据 profile 过滤）
fn build_subagent_tools(
    profile: &subagent::SubagentProfile,
    project_root: &str,
) -> (Vec<ToolSpec>, HashMap<String, String>) {
    // 注入和主 Agent 相同的内置工具，但在执行时按 profile 过滤
    let mut tools: Vec<ToolSpec> = Vec::new();
    let mut tool_map: HashMap<String, String> = HashMap::new();

    // 始终注入的文件工具（但 profile 会限制哪些可执行）
    for spec in file_tools::get_file_tool_specs() {
        if profile.is_tool_allowed(&spec.function.name) {
            tool_map.insert(spec.function.name.clone(), "__builtin__".into());
            tools.push(spec);
        }
    }
    // 命令执行工具
    if profile.is_tool_allowed("execute_command") {
        let spec = shell_tools::get_command_tool_spec();
        tool_map.insert(spec.function.name.clone(), "__builtin__".into());
        tools.push(spec);
    }
    // Web 工具
    for spec in web_tools::get_web_tool_specs() {
        if profile.is_tool_allowed(&spec.function.name) {
            tool_map.insert(spec.function.name.clone(), "__builtin__".into());
            tools.push(spec);
        }
    }
    // Git 工具
    for spec in git_tools::get_git_tool_specs() {
        if profile.is_tool_allowed(&spec.function.name) && is_git_repo(project_root) {
            tool_map.insert(spec.function.name.clone(), "__builtin__".into());
            tools.push(spec);
        }
    }
    // LSP 工具（所有 5 个）
    for spec in lsp_agent_tools::get_all_lsp_tool_specs() {
        if profile.is_tool_allowed(&spec.function.name) {
            tool_map.insert(spec.function.name.clone(), "__builtin__".into());
            tools.push(spec);
        }
    }

    (tools, tool_map)
}

/// Helper: check if a directory is a git repo
fn is_git_repo(dir: &str) -> bool {
    std::path::Path::new(dir).join(".git").exists()
}

/// 截断过长的工具返回内容，防止 LLM 上下文膨胀。完整内容保留在 agentSteps 中供用户查看。
/// 头尾保留式截断：前 60% + 后 40%（20k 字符 ≈ 5-10k tokens），避免只留头部把
/// read_file 已保留的文件首尾再次砍掉，导致探索看不到关键内容。
fn truncate_tool_result(s: &str) -> String {
    // 空结果统一替换为极短占位，避免垃圾进上下文（参考 oh-my-pi 的 useless-result elision）
    if s.trim().is_empty() {
        return "[无结果]".to_string();
    }
    const MAX_LEN: usize = 20_000;
    if s.len() <= MAX_LEN {
        s.to_string()
    } else {
        let head = (MAX_LEN * 6) / 10;
        let tail = MAX_LEN - head;
        let head_end = s.floor_char_boundary(head.min(s.len()));
        let tail_start = s.floor_char_boundary(s.len().saturating_sub(tail));
        if tail_start > head_end {
            format!(
                "{}\n\n... [已截断: 共{}字符，保留首尾] ...\n\n{}",
                &s[..head_end],
                s.len(),
                &s[tail_start..]
            )
        } else {
            let safe_end = s.floor_char_boundary(MAX_LEN.min(s.len()));
            format!("{}\n\n... [已截断: 共{}字符]", &s[..safe_end], s.len())
        }
    }
}

/// 头部+尾部截断：保留前 60% 与后 40% 字符，中间以省略标记分隔。
/// 用于工作流步骤上下文等「需要保留首尾关键信息」的拼接场景。
fn truncate_head_tail(s: &str, max_chars: usize) -> String {
    let total = s.chars().count();
    if total <= max_chars {
        return s.to_string();
    }
    let head = (max_chars * 6) / 10;
    let tail = max_chars - head;
    let head_str: String = s.chars().take(head).collect();
    let tail_str: String = s.chars().skip(total - tail).collect();
    format!(
        "{}\n… [已省略 {} 字符] …\n{}",
        head_str,
        total - head - tail,
        tail_str
    )
}

/// 估算单条 API 消息的 token 数（tiktoken 本地计数；失败时回退到字符数/2）。
/// 用于上下文压缩触发与保留预算的判定。
fn estimate_message_tokens(model: &str, msg: &serde_json::Value) -> usize {
    let s = serde_json::to_string(msg).unwrap_or_default();
    crate::utils::token_counter::count_tokens(model, &s).unwrap_or(s.len() / 2)
}

/// 上下文 token 估算（带粗估预检）：先用 字符数/3 粗估，远低于预算时直接返回，
/// 避免每轮对全部消息做 tiktoken 编码拖慢 Agent 循环；接近预算时才精确统计。
fn estimate_context_tokens(model: &str, messages: &[serde_json::Value], budget: usize) -> usize {
    let rough: usize = messages
        .iter()
        .map(|m| serde_json::to_string(m).map(|s| s.len() / 3).unwrap_or(0))
        .sum();
    if rough < budget.saturating_sub(10_000) {
        return rough;
    }
    messages
        .iter()
        .map(|m| estimate_message_tokens(model, m))
        .sum()
}

/// 判断 API 错误是否由上下文超限导致（用于溢出自动恢复：压缩后重试一次）。
fn is_context_overflow_error(e: &str) -> bool {
    let e = e.to_lowercase();
    [
        "context length",
        "context_length",
        "context window",
        "maximum context",
        "max context",
        "token limit",
        "too many tokens",
        "tokens exceeded",
        "input is too long",
        "prompt is too long",
        "exceeds the maximum",
        "ctx_len",
        "requested tokens",
        "maximum input tokens",
    ]
    .iter()
    .any(|k| e.contains(k))
}

/// read_skill 工具的 ToolSpec：让模型按需读取已启用 Skill 的完整说明。
/// 配合前端「技能目录」注入，避免把每个 Skill 的全量内容随每次请求发送。
pub fn skill_read_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: ToolFunctionSpec {
            name: "read_skill".into(),
            description: "读取一个已启用 Skill 的完整说明与使用指令。当任务涉及某个技能（如文档处理、表格、演示文稿等）时，先用此工具获取其完整内容再执行。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {"type": "string", "description": "技能名称（如 docx、xlsx）"}
                },
                "required": ["name"]
            }),
        },
    }
}

/// 执行 read_skill：按名称（不区分大小写）或 ID 查找技能并返回完整内容。
fn execute_skill_read(skills: &[SkillConfig], name: &str) -> Result<ToolResult, String> {
    if name.is_empty() {
        return Err("read_skill 缺少 name 参数".to_string());
    }
    if let Some(skill) = skills
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name) || s.id == name)
    {
        Ok(file_tools::tool_ok(format!(
            "[Skill: {}]\n{}",
            skill.name, skill.content
        )))
    } else {
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        Err(format!(
            "未找到技能: {name}；可用技能: {}",
            names.join(", ")
        ))
    }
}

/// read_artifact 工具的 ToolSpec：读取压缩时本地归档的历史档案（snapcompact 式）。
/// 摘要生成失败或需要恢复早期细节时，模型按 [历史归档 #id] 占位中的 id 调用本工具。
pub fn artifact_read_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".into(),
        function: ToolFunctionSpec {
            name: "read_artifact".into(),
            description: "读取本地历史归档的完整内容（压缩早期对话时自动归档；需要早期细节时按 [历史归档 #id] 占位中的 id 调用）。".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "artifact_id": {"type": "string", "description": "历史归档 ID（如 a1b2c3d4…）"}
                },
                "required": ["artifact_id"]
            }),
        },
    }
}

/// 执行 read_artifact：读取 {app_data}/archives/{id}.txt 并截断返回。
fn execute_artifact_read(app: &AppHandle, artifact_id: &str) -> Result<ToolResult, String> {
    if artifact_id.is_empty() {
        return Err("read_artifact 缺少 artifact_id 参数".to_string());
    }
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取数据目录失败: {e}"))?
        .join("archives");
    let path = dir.join(format!("{}.txt", artifact_id));
    if !path.exists() {
        return Err(format!(
            "未找到历史归档 #{artifact_id}（档案已清理或目录不存在）"
        ));
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    Ok(file_tools::tool_ok(format!(
        "[历史归档 #{}]\n{}",
        artifact_id,
        truncate_head_tail(&content, 30_000)
    )))
}

/// 从子智能体上下文提取已完成工作的进度摘要（命中轮数上限或未产出最终文本时使用），
/// 保证即使任务被截断也能向主 Agent 返回有效信息。
fn build_subagent_progress_summary(sub_msgs: &[serde_json::Value], max_rounds: usize) -> String {
    let mut last_assistant_text: Option<String> = None;
    let mut summary_text: Option<String> = None;
    let mut files: Vec<String> = Vec::new();
    let mut tool_count: usize = 0;

    for msg in sub_msgs {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "assistant" {
            if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
                if !s.trim().is_empty() {
                    last_assistant_text = Some(s.to_string());
                }
            }
            if let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                tool_count += tcs.len();
                for tc in tcs {
                    if let Some(args) = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|v| v.as_str())
                    {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(args) {
                            if let Some(p) = v.get("path").and_then(|x| x.as_str()) {
                                if !files.contains(&p.to_string()) {
                                    files.push(p.to_string());
                                }
                            }
                        }
                    }
                }
            }
        } else if role == "system" {
            if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
                if s.contains("历史摘要") || s.contains("历史归档") {
                    summary_text = Some(s.to_string());
                }
            }
        }
    }

    let last_text = last_assistant_text
        .map(|s| {
            let t = &s[..s.floor_char_boundary(s.len().min(600))];
            t.to_string()
        })
        .unwrap_or_else(|| "（无中间文本输出）".to_string());
    let summary_part = match summary_text {
        Some(s) => format!(
            "\n\n压缩摘要（早期发现）：{}\n",
            truncate_head_tail(&s, 800)
        ),
        None => String::new(),
    };
    let file_list = if files.is_empty() {
        "（未记录）".to_string()
    } else {
        files
            .iter()
            .map(|f| format!("- {}", f))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "[已达子智能体最大轮数限制（{} 轮），任务未完成。已完成工作如下（共 {} 次工具调用）：\n\n最后输出：{}\n{}涉及文件：\n{}\n\n请基于以上进度继续或让用户决定下一步。]",
        max_rounds, tool_count, last_text, summary_part, file_list
    )
}
/// 本地归档早期消息（snapcompact 式零成本压缩回退）：
/// 写入 {app_data}/archives/{uuid}.txt，返回 [历史归档 #id] 占位文本（含条数/token 估算/涉及工具）。
fn archive_early_messages(
    window: &Window,
    early: &[serde_json::Value],
    model: &str,
) -> Result<String, String> {
    let app = window.app_handle();
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取数据目录失败: {e}"))?
        .join("archives");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let artifact_id = uuid::Uuid::new_v4().simple().to_string();

    let mut out = String::new();
    let mut total_tokens: usize = 0;
    let mut tool_names: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for msg in early {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("?");
        let text = match msg.get("content") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        let text = &text[..text.floor_char_boundary(text.len().min(2000))];
        total_tokens += estimate_message_tokens(model, msg);
        if let Some(name) = msg.get("name").and_then(|v| v.as_str()) {
            tool_names.insert(name.to_string());
        }
        out.push_str(&format!("[{}]: {}\n", role, text));
    }
    let path = dir.join(format!("{}.txt", artifact_id));
    std::fs::write(&path, &out).map_err(|e| e.to_string())?;

    let names = if tool_names.is_empty() {
        String::new()
    } else {
        format!(
            "涉及工具: {}。",
            tool_names.into_iter().collect::<Vec<_>>().join(", ")
        )
    };
    Ok(format!(
        "[历史归档 #{}] 早期对话已本地归档（{} 条消息，约 {} tokens）。{}需要细节时用 read_artifact 工具读取。",
        artifact_id,
        early.len(),
        total_tokens,
        names
    ))
}
/// 生成工作交接文档（handoff）：写入 {app_data}/handoffs/{assistant}-{topic}-{ts}.md。
/// 内容：时间/状态、已完成内容（截断）、涉及文件（从工具调用参数提取）、未完成原因。
fn write_handoff_doc(
    app: &AppHandle,
    assistant_id: &str,
    topic_id: &str,
    accumulated: &str,
    messages: &[serde_json::Value],
    final_error: &Option<String>,
) -> Result<String, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取数据目录失败: {e}"))?
        .join("handoffs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    // 收集涉及文件（从 assistant 消息的 tool_calls 参数中提取 path）
    let mut files: Vec<String> = Vec::new();
    for msg in messages {
        if let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) {
            for tc in tcs {
                let name = tc
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if [
                    "read_file",
                    "write_file",
                    "replace_in_file",
                    "delete_file",
                    "make_directory",
                ]
                .contains(&name)
                {
                    if let Some(args) = tc
                        .get("function")
                        .and_then(|f| f.get("arguments"))
                        .and_then(|v| v.as_str())
                    {
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(args) {
                            if let Some(p) = v.get("path").and_then(|x| x.as_str()) {
                                if !files.contains(&p.to_string()) {
                                    files.push(p.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let safe_a: String = assistant_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(12)
        .collect();
    let safe_t: String = topic_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(12)
        .collect();
    let path = dir.join(format!("{}-{}-{}.md", safe_a, safe_t, ts));

    let files_section = if files.is_empty() {
        "（无）".to_string()
    } else {
        files
            .iter()
            .map(|f| format!("- {}", f))
            .collect::<Vec<_>>()
            .join(
                "
",
            )
    };
    let body = format!(
        "# 工作交接文档（未完成任务）

- 时间: {}
- 助手: {}
- 话题: {}
- 错误: {}

## 已完成内容

{}

## 涉及文件

{}

## 下一步

请基于以上状态继续未完成的工作。
",
        ts,
        assistant_id,
        topic_id,
        final_error.as_deref().unwrap_or("无"),
        truncate_head_tail(accumulated, 4000),
        files_section
    );
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    Ok(path.display().to_string())
}
/// 为文件修改工具预计算 diff 预览（审批前展示用）。
/// 返回 (preview_diff, file_path)。
fn compute_diff_preview(
    tool_name: &str,
    arguments: &serde_json::Value,
    project_root: &str,
) -> (Option<String>, Option<String>) {
    let file_path = arguments["path"].as_str().map(|s| s.to_string());
    let full_path = || {
        let root = std::path::Path::new(project_root);
        file_path.as_ref().map(|p| root.join(p))
    };

    match tool_name {
        "write_file" => {
            let new_content = arguments["content"].as_str().unwrap_or("");
            let old_content = full_path()
                .and_then(|p| std::fs::read_to_string(&p).ok())
                .unwrap_or_default();
            let diff = simple_diff(
                &old_content,
                new_content,
                &file_path.clone().unwrap_or_default(),
            );
            (Some(diff), file_path)
        }
        "replace_in_file" => {
            let old_s = arguments["old_string"].as_str().unwrap_or("");
            let new_s = arguments["new_string"].as_str().unwrap_or("");
            let old_content = full_path()
                .and_then(|p| std::fs::read_to_string(&p).ok())
                .unwrap_or_default();
            if old_content.contains(old_s) {
                let new_content = old_content.replacen(old_s, new_s, 1);
                let diff = simple_diff(
                    &old_content,
                    &new_content,
                    &file_path.clone().unwrap_or_default(),
                );
                (Some(diff), file_path)
            } else {
                (None, file_path)
            }
        }
        "delete_file" => {
            let old_content = full_path()
                .and_then(|p| std::fs::read_to_string(&p).ok())
                .unwrap_or_default();
            if old_content.is_empty() {
                (None, file_path)
            } else {
                let diff = simple_diff(&old_content, "", &file_path.clone().unwrap_or_default());
                (Some(diff), file_path)
            }
        }
        _ => (None, None),
    }
}

/// 简易 unified diff 生成器：比较 old 和 new 文本，输出 Git 风格的 diff。
fn simple_diff(old: &str, new: &str, file_name: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    // 找共同前缀
    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    // 找共同后缀（在去除前缀后）
    let mut suffix = 0;
    while suffix < old_lines.len().saturating_sub(prefix)
        && suffix < new_lines.len().saturating_sub(prefix)
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_start = prefix + 1;
    let old_end = old_lines.len() - suffix;
    let new_start = prefix + 1;
    let new_end = new_lines.len() - suffix;

    let added = (new_end as i64 - new_start as i64 + 1).max(0);
    let removed = (old_end as i64 - old_start as i64 + 1).max(0);

    let mut diff = format!(
        "--- a/{}\n+++ b/{}\n@@ -{},{} +{},{} @@\n",
        file_name,
        file_name,
        old_start,
        removed.max(1),
        new_start,
        added.max(1),
    );

    // 上下文（前3行）
    let ctx_start = prefix.saturating_sub(3);
    for line in &old_lines[ctx_start..prefix] {
        diff.push_str(&format!(" {}\n", line));
    }

    // 删除的行
    for line in &old_lines[prefix..old_end] {
        diff.push_str(&format!("-{}\n", line));
    }
    // 新增的行
    for line in &new_lines[prefix..new_end] {
        diff.push_str(&format!("+{}\n", line));
    }

    // 上下文（后3行）
    let ctx_end = (new_end + 3).min(new_lines.len());
    for line in &new_lines[new_end..ctx_end] {
        diff.push_str(&format!(" {}\n", line));
    }

    if diff.len() > 8000 {
        diff.truncate(8000);
        diff.push_str("\n... [diff 已截断]");
    }
    diff
}

/// 判断工具错误是否可重试（网络类错误可重试，权限/参数错误不可重试）
fn is_retryable_error(e: &str) -> bool {
    let lower = e.to_lowercase();
    lower.contains("timeout")
        || lower.contains("connection refused")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
        || lower.contains("eof")
        || lower.contains("not connected")
        || lower.contains("tls")
        || lower.contains("dns")
}

/// 带自动重试的工具执行包装器（仅对 MCP 网络调用使用）。
async fn execute_tool_with_retry<F, Fut>(
    _tool_name: &str,
    execute: F,
    config: &crate::core::models::AppConfig,
    token: &tokio_util::sync::CancellationToken,
) -> Result<ToolResult, String>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<ToolResult, String>>,
{
    if !config.auto_retry_enabled {
        return execute().await;
    }
    let max_attempts = config.auto_retry_count as usize + 1;
    let mut last_error = None;
    for attempt in 1..=max_attempts {
        match execute().await {
            Ok(result) => {
                if result.is_error && attempt < max_attempts {
                    if let Some(err_text) = result
                        .content
                        .first()
                        .and_then(|c| c.data.get("text"))
                        .and_then(|v| v.as_str())
                    {
                        if is_retryable_error(err_text) {
                            tokio::select! {
                                _ = token.cancelled() => return Err("cancelled".into()),
                                _ = tokio::time::sleep(std::time::Duration::from_millis(config.auto_retry_delay_ms)) => {}
                            }
                            continue;
                        }
                    }
                }
                return Ok(result);
            }
            Err(e) => {
                last_error = Some(e.clone());
                if attempt < max_attempts && is_retryable_error(&e) {
                    tokio::select! {
                        _ = token.cancelled() => return Err("cancelled".into()),
                        _ = tokio::time::sleep(std::time::Duration::from_millis(config.auto_retry_delay_ms)) => {}
                    }
                    continue;
                }
                return Err(e);
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "auto-retry exhausted".into()))
}

/// 处理 delegate_task 工具调用（从主 Agent 循环中调用）。
///
/// 解析参数、验证权限、获取 profile，然后委托给 execute_subagent 执行。
#[allow(
    clippy::too_many_arguments,
    reason = "agent delegation keeps inherited execution context explicit"
)]
async fn handle_delegate_task(
    window: &Window,
    app: &AppHandle,
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    parent_assistant_id: &str,
    parent_topic_id: &str,
    arguments: &serde_json::Value,
    project_id: Option<&str>,
    agent_mode: &AgentMode,
    semaphore: Arc<Semaphore>,
    token: &CancellationToken,
    profile_model_overrides: &[ProfileModelOverride],
    custom_profiles: &[crate::core::models::CustomSubagentProfile],
    locale: &str,
) -> Result<ToolResult, String> {
    // AGT-11：单发/后台 delegate_task 也受全局并发上限约束（与批量 delegate_tasks 共享同一信号量）。
    let _permit = semaphore
        .acquire()
        .await
        .map_err(|_| "并发控制信号量已关闭".to_string())?;

    // 解析参数
    let profile_id = arguments["profile"].as_str().unwrap_or("general");
    let task_desc = arguments["task"].as_str().unwrap_or("").to_string();
    if task_desc.is_empty() {
        return Err("delegate_task 缺少必填参数 'task'".into());
    }
    let profile = subagent::find_profile(profile_id, custom_profiles)
        .ok_or_else(|| format!("未知的子智能体类型: '{}'，可用: explorer, coder, general, architect, debugger, reviewer, writer, tester, requirements, 或自定义角色 ID", profile_id))?;

    let context_files: Vec<String> = arguments["context_files"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // 解析项目根目录
    let project_root = file_tools::resolve_project_root(app, project_id)?;

    // 权限检查：非 Auto 模式下需要用户确认
    if *agent_mode != AgentMode::Auto {
        let custom_rules = permission::load_permissions(Some(&project_root)).rules;
        let action = permission::check_permission(
            "delegate_task",
            "__aio-filesystem__",
            arguments,
            agent_mode,
            &custom_rules,
        );
        match action {
            PermissionAction::Deny => {
                return Err("delegate_task 在当前模式下被安全策略禁止".into());
            }
            PermissionAction::Ask => {
                let pending = app.state::<PendingApprovals>();
                let reason = format!(
                    "启动子智能体\n\n类型: {}\n任务: {}\n\n确认？",
                    profile.name,
                    truncate_for_display(&task_desc, 150),
                );
                let approval_fut = crate::commands::mcp::request_tool_approval(
                    app,
                    pending.inner(),
                    "__aio-filesystem__",
                    "delegate_task",
                    arguments,
                    &reason,
                    None,
                    None,
                );
                tokio::select! {
                    _ = token.cancelled() => return Err("cancelled".into()),
                    res = approval_fut => res?,
                }
            }
            PermissionAction::Allow => {}
        }
    }

    // 执行子 Agent
    execute_single_delegate(
        window,
        app,
        client,
        api_url,
        api_key,
        model,
        parent_assistant_id,
        parent_topic_id,
        profile_id,
        &task_desc,
        &context_files,
        None, // extra_context — 单数调用不注入共享上下文
        project_id,
        agent_mode,
        token,
        profile_model_overrides,
        custom_profiles,
        locale,
    )
    .await
}

/// 单个子智能体执行的核心逻辑（被 `handle_delegate_task` 和 `handle_delegate_tasks` 共用）。
///
/// 负责：解析 profile、模型覆盖、项目根目录，然后调用 execute_subagent。
/// 不包含权限检查——由调用方负责。
#[allow(
    clippy::too_many_arguments,
    reason = "agent delegation keeps inherited execution context explicit"
)]
async fn execute_single_delegate(
    window: &Window,
    app: &AppHandle,
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    parent_assistant_id: &str,
    parent_topic_id: &str,
    profile_id: &str,
    task_desc: &str,
    context_files: &[String],
    extra_context: Option<&str>,
    project_id: Option<&str>,
    agent_mode: &AgentMode,
    token: &CancellationToken,
    profile_model_overrides: &[ProfileModelOverride],
    custom_profiles: &[crate::core::models::CustomSubagentProfile],
    locale: &str,
) -> Result<ToolResult, String> {
    let profile = subagent::find_profile(profile_id, custom_profiles)
        .ok_or_else(|| format!("未知的子智能体类型: '{}'，可用: explorer, coder, general, architect, debugger, reviewer, writer, tester, requirements, 或自定义角色 ID", profile_id))?;

    // 模型解析：per-profile override 优先，否则使用主模型
    let override_info = profile_model_overrides
        .iter()
        .find(|o| o.profile_id == profile.id);
    let (resolved_api_url, resolved_api_key, resolved_model) = if let Some(ov) = override_info {
        (
            ov.api_url.as_str(),
            ov.api_key.as_str(),
            ov.model_id.as_str(),
        )
    } else {
        (api_url, api_key, model)
    };

    // 解析项目根目录
    let project_root = file_tools::resolve_project_root(app, project_id)?;

    execute_subagent(
        window,
        app,
        client,
        resolved_api_url,
        resolved_api_key,
        resolved_model,
        parent_assistant_id,
        parent_topic_id,
        &profile,
        task_desc,
        context_files,
        extra_context,
        &project_root,
        project_id,
        agent_mode,
        token,
        locale,
    )
    .await
}

/// 处理 delegate_tasks 批量工具调用。
///
/// 解析 tasks 数组和可选的 context，整体检查一次权限，
/// 然后使用 FuturesUnordered 并行执行所有子智能体。
/// 返回所有子任务结果的汇总报告。
#[allow(
    clippy::too_many_arguments,
    reason = "parallel delegation keeps inherited execution context explicit"
)]
async fn handle_delegate_tasks(
    window: &Window,
    app: &AppHandle,
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    parent_assistant_id: &str,
    parent_topic_id: &str,
    arguments: &serde_json::Value,
    project_id: Option<&str>,
    agent_mode: &AgentMode,
    token: &CancellationToken,
    profile_model_overrides: &[ProfileModelOverride],
    custom_profiles: &[crate::core::models::CustomSubagentProfile],
    semaphore: Arc<Semaphore>,
    locale: &str,
) -> Result<ToolResult, String> {
    // 解析 tasks 数组
    let tasks_array = arguments["tasks"]
        .as_array()
        .ok_or_else(|| "delegate_tasks 缺少必填参数 'tasks'（应为数组）".to_string())?;

    if tasks_array.is_empty() {
        return Err("delegate_tasks: 'tasks' 数组不能为空".into());
    }
    if tasks_array.len() > 10 {
        return Err("delegate_tasks: 'tasks' 数组最多 10 个元素".into());
    }

    let shared_context = arguments["context"]
        .as_str()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty());

    // 验证每个 task 的结构
    for (idx, task_obj) in tasks_array.iter().enumerate() {
        let profile = task_obj["profile"].as_str();
        let task = task_obj["task"].as_str();
        if profile.is_none() || profile.unwrap().is_empty() {
            return Err(format!(
                "delegate_tasks: 第 {} 个 task 缺少必填字段 'profile'",
                idx + 1
            ));
        }
        if task.is_none() || task.unwrap().is_empty() {
            return Err(format!(
                "delegate_tasks: 第 {} 个 task 缺少必填字段 'task'",
                idx + 1
            ));
        }
    }

    // 权限检查：整个批量调用一次审批（非 Auto 模式）
    if *agent_mode != AgentMode::Auto {
        // 解析项目根目录用于加载权限规则
        let project_root = file_tools::resolve_project_root(app, project_id)?;
        let custom_rules = permission::load_permissions(Some(&project_root)).rules;
        let action = permission::check_permission(
            "delegate_tasks",
            "__aio-filesystem__",
            arguments,
            agent_mode,
            &custom_rules,
        );
        match action {
            PermissionAction::Deny => {
                return Err("delegate_tasks 在当前模式下被安全策略禁止".into());
            }
            PermissionAction::Ask => {
                let pending = app.state::<PendingApprovals>();
                let profiles_summary: Vec<String> = tasks_array
                    .iter()
                    .map(|t| {
                        format!(
                            "  - {}: {}",
                            t["profile"].as_str().unwrap_or("?"),
                            truncate_for_display(t["task"].as_str().unwrap_or(""), 80)
                        )
                    })
                    .collect();
                let reason = format!(
                    "批量启动 {} 个子智能体:\n{}\n\n确认？",
                    tasks_array.len(),
                    profiles_summary.join("\n"),
                );
                let approval_fut = crate::commands::mcp::request_tool_approval(
                    app,
                    pending.inner(),
                    "__aio-filesystem__",
                    "delegate_tasks",
                    arguments,
                    &reason,
                    None,
                    None,
                );
                tokio::select! {
                    _ = token.cancelled() => return Err("cancelled".into()),
                    res = approval_fut => res?,
                }
            }
            PermissionAction::Allow => {}
        }
    }

    // 并发执行所有子任务
    let mut unordered = FuturesUnordered::new();

    for task_obj in tasks_array.iter() {
        let profile_id = task_obj["profile"]
            .as_str()
            .unwrap_or("general")
            .to_string();
        let task_desc = task_obj["task"].as_str().unwrap_or("").to_string();
        let task_context_files: Vec<String> = task_obj["context_files"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let extra_ctx = shared_context.clone();

        let window_clone = window.clone();
        let app_clone = app.clone();
        let client_clone = client.clone();
        let api_url_clone = api_url.to_string();
        let api_key_clone = api_key.to_string();
        let model_clone = model.to_string();
        let parent_aid = parent_assistant_id.to_string();
        let parent_tid = parent_topic_id.to_string();
        let pid_clone = project_id.map(|s| s.to_string());
        let token_clone = token.clone();
        let overrides_clone = profile_model_overrides.to_vec();
        let customs_clone = custom_profiles.to_vec();
        let sem_clone = semaphore.clone();
        let locale_clone = locale.to_string();
        let mode_clone = agent_mode.clone();

        unordered.push(async move {
            let _permit = sem_clone
                .acquire()
                .await
                .map_err(|_| "并发控制信号量已关闭".to_string())?;

            execute_single_delegate(
                &window_clone,
                &app_clone,
                &client_clone,
                &api_url_clone,
                &api_key_clone,
                &model_clone,
                &parent_aid,
                &parent_tid,
                &profile_id,
                &task_desc,
                &task_context_files,
                extra_ctx.as_deref(),
                pid_clone.as_deref(),
                &mode_clone,
                &token_clone,
                &overrides_clone,
                &customs_clone,
                &locale_clone,
            )
            .await
        });
    }

    // 收集所有结果
    let mut successes: Vec<String> = Vec::new();
    let mut failures: Vec<String> = Vec::new();

    while let Some(result) = unordered.next().await {
        match result {
            Ok(tr) => {
                let summary = tr
                    .content
                    .iter()
                    .find(|c| c.kind == "text")
                    .and_then(|c| c.data.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("(done)");
                successes.push(summary.to_string());
            }
            Err(e) => {
                failures.push(e);
            }
        }
    }

    // 构造汇总报告
    let total = successes.len() + failures.len();
    let mut report = format!("[批量委托完成] 共 {} 个子任务", total);

    if !successes.is_empty() {
        report.push_str(&format!("\n\n成功 ({}):", successes.len()));
        for (i, s) in successes.iter().enumerate() {
            report.push_str(&format!("\n--- 子任务 {} ---\n{}\n", i + 1, s));
        }
    }
    if !failures.is_empty() {
        report.push_str(&format!("\n\n失败 ({}):", failures.len()));
        for (i, f) in failures.iter().enumerate() {
            report.push_str(&format!("\n- 子任务 {} 错误: {}", i + 1, f));
        }
    }

    Ok(ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": report}),
        }],
        is_error: false,
    })
}
/// 执行一个工作流（按顺序依次运行每个步骤的子智能体）。
///
/// 每个步骤独立调用 `handle_delegate_task`，前一步的输出作为上下文追加到下一步的任务描述中。
/// 通过 Tauri 事件向前端推送工作流进度（workflow-start / workflow-step-start / workflow-step-complete / workflow-complete）。
#[allow(
    clippy::too_many_arguments,
    reason = "workflow orchestration keeps inherited execution context explicit"
)]
async fn execute_workflow(
    window: &Window,
    app: &AppHandle,
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    parent_assistant_id: &str,
    parent_topic_id: &str,
    workflow: &Workflow,
    project_id: Option<&str>,
    semaphore: Arc<Semaphore>,
    custom_profiles: &[CustomSubagentProfile],
    profile_model_overrides: &[ProfileModelOverride],
    token: &CancellationToken,
    locale: &str,
) -> Result<ToolResult, String> {
    let total_steps = workflow.steps.len();

    // 发射 workflow-start 事件
    let _ = window.emit(
        "workflow-start",
        serde_json::json!({
            "workflowId": workflow.workflow_id,
            "title": workflow.title,
            "steps": workflow.steps.iter().map(|s| serde_json::json!({
                "stepId": s.step_id,
                "profileId": s.profile_id,
                "name": s.name,
                "status": "pending",
            })).collect::<Vec<_>>(),
        }),
    );

    let mut context = String::new();
    let mut all_results: Vec<String> = Vec::new();

    for (idx, step) in workflow.steps.iter().enumerate() {
        if token.is_cancelled() {
            let _ = window.emit(
                "workflow-complete",
                serde_json::json!({ "workflowId": workflow.workflow_id }),
            );
            return Err("工作流被用户取消".into());
        }

        // 发射 workflow-step-start 事件
        let _ = window.emit(
            "workflow-step-start",
            serde_json::json!({ "stepId": step.step_id }),
        );

        let start = std::time::Instant::now();

        // 构建任务描述：基础任务 + 上下文
        let mut task_desc = step.task_description.clone();
        if !context.is_empty() {
            task_desc.push_str("\n\n== 前期工作上下文 ==\n");
            task_desc.push_str(&context);
        }

        // 构建 arguments JSON（与 delegate_task 格式一致）
        let arguments = serde_json::json!({
            "profile": step.profile_id,
            "task": task_desc,
        });

        // 调用 handle_delegate_task 执行该步骤
        let result = handle_delegate_task(
            window,
            app,
            client,
            api_url,
            api_key,
            model,
            parent_assistant_id,
            parent_topic_id,
            &arguments,
            project_id,
            &AgentMode::Auto, // 工作流步骤以 Auto 模式执行（无需用户逐个确认）
            semaphore.clone(),
            token,
            profile_model_overrides,
            custom_profiles,
            locale,
        )
        .await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(tr) => {
                let step_result_text = tr
                    .content
                    .iter()
                    .find(|c| c.kind == "text")
                    .and_then(|c| c.data.get("text"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("(done)")
                    .to_string();

                // 追加到上下文（头部+尾部截断，防止多步工作流的上下文无限累积）
                context.push_str(&format!(
                    "\n## 步骤 {} ({}): {}\n{}",
                    idx + 1,
                    step.name,
                    step.profile_id,
                    truncate_head_tail(&step_result_text, 1200)
                ));
                all_results.push(format!(
                    "步骤 {} ({}): {}",
                    idx + 1,
                    step.name,
                    step_result_text
                ));

                // 发射 workflow-step-complete 事件
                let _ = window.emit(
                    "workflow-step-complete",
                    serde_json::json!({
                        "stepId": step.step_id,
                        "status": "completed",
                        "duration": duration_ms,
                    }),
                );
            }
            Err(e) => {
                context.push_str(&format!(
                    "\n## 步骤 {} ({}): [失败] {}",
                    idx + 1,
                    step.name,
                    e
                ));
                all_results.push(format!("步骤 {} ({}): [失败] {}", idx + 1, step.name, e));

                // 发射 workflow-step-complete 事件（失败）
                let _ = window.emit(
                    "workflow-step-complete",
                    serde_json::json!({
                        "stepId": step.step_id,
                        "status": "failed",
                        "duration": duration_ms,
                    }),
                );

                // 步骤失败不中断整个工作流，继续执行后续步骤
            }
        }
    }

    // 发射 workflow-complete 事件
    let _ = window.emit(
        "workflow-complete",
        serde_json::json!({ "workflowId": workflow.workflow_id }),
    );

    // 构建最终结果
    let summary = format!(
        "工作流 '{}' 执行完成（{}/{} 步骤）\n\n{}",
        workflow.title,
        total_steps,
        total_steps,
        all_results.join("\n\n")
    );

    Ok(ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: serde_json::json!({ "text": summary }),
        }],
        is_error: false,
    })
}

/// 执行子 Agent 循环（从主 Agent 的工具执行循环中调用）。
///
/// 创建独立的 LLM 上下文，注入子 Agent 系统提示词 + 任务描述，
/// 运行最多 `max_rounds` 轮流式调用 + 工具执行，完成后返回结果总结。
#[allow(
    clippy::too_many_arguments,
    reason = "subagent execution keeps inherited parent context explicit"
)]
async fn execute_subagent(
    window: &Window,
    app: &AppHandle,
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    parent_assistant_id: &str,
    parent_topic_id: &str,
    profile: &subagent::SubagentProfile,
    task_desc: &str,
    context_files: &[String],
    extra_context: Option<&str>,
    project_root: &str,
    project_id: Option<&str>,
    agent_mode: &AgentMode,
    token: &CancellationToken,
    locale: &str,
) -> Result<ToolResult, String> {
    let subagent_id = uuid::Uuid::new_v4().to_string();
    let model_to_use = profile.model_override.as_deref().unwrap_or(model);

    // 通知前端：子 Agent 已启动
    let _ = window.emit(
        "subagent-start",
        SubagentStartPayload {
            parent_assistant_id: parent_assistant_id.to_string(),
            parent_topic_id: parent_topic_id.to_string(),
            subagent_id: subagent_id.clone(),
            profile_id: profile.id.clone(),
            profile_name: profile.name.clone(),
            task_summary: truncate_for_display(task_desc, 150),
        },
    );

    let mut sub_msgs = build_subagent_messages(
        profile,
        task_desc,
        context_files,
        project_root,
        extra_context,
        locale,
    );
    let (sub_tools, _sub_tool_map) = build_subagent_tools(profile, project_root);
    let tools_slice: Option<&[ToolSpec]> = if sub_tools.is_empty() {
        None
    } else {
        Some(&sub_tools)
    };

    let mut round: usize = 0;
    let mut final_text = String::new();
    // 输出截断（finish_reason="length"）续写重试次数（最多 1 次）
    let mut length_retries: u32 = 0;
    // 上下文溢出自动恢复次数（压缩后重试）
    let mut sub_overflow_retries: u32 = 0;
    // 子 Agent 轮数上限：对齐 oh-my-pi「不设低轮数上限、靠上下文管理兜底」的思路，
    // 100 轮仅作失控保护（防模型死循环），正常长任务（如大规模探索）不受限；
    // 命中上限时返回已完成的进度摘要而非空结果。
    const MAX_SUBAGENT_ROUNDS: usize = 100;
    // 子 Agent 上下文安全阀预算（token）：超限触发压缩，无需模型窗口信息
    const SUBAGENT_CONTEXT_BUDGET: usize = 45_000;

    loop {
        round += 1;
        if token.is_cancelled() {
            break;
        }
        if round > MAX_SUBAGENT_ROUNDS {
            // 命中失控保护：把已完成的工作整理成有效返回，而不是丢弃
            final_text = build_subagent_progress_summary(&sub_msgs, MAX_SUBAGENT_ROUNDS);
            break;
        }

        // 流式调用前：发射心跳告知前端子 Agent 工作中
        let _ = window.emit(
            "subagent-step",
            SubagentStepPayload {
                parent_assistant_id: parent_assistant_id.to_string(),
                parent_topic_id: parent_topic_id.to_string(),
                subagent_id: subagent_id.clone(),
                round: round as u32,
                step_type: "streaming".to_string(),
                summary: format!("第 {} 轮 LLM 调用中...", round),
            },
        );

        // 流式调用一轮
        let result = stream_one_round(
            window,
            token,
            client,
            api_url.to_string(),
            api_key,
            model_to_use,
            &sub_msgs,
            tools_slice,
            parent_assistant_id,
            parent_topic_id,
            true, // suppress events for sub-agent (uses subagent-* events instead)
            None, // 子 Agent 图像输出 v1 不支持：不入盘、不重写，仅保留文本
            false,
            None, // 子 Agent 输出上限由 round cap 兜底，不额外注入 max_tokens
        )
        .await;

        match result {
            Ok(rr) => {
                // 追加 assistant 消息到子 Agent 上下文
                let mut asst_obj = serde_json::Map::new();
                asst_obj.insert("role".into(), json!("assistant"));
                asst_obj.insert("content".into(), rr.api_content.clone());
                if !rr.tool_calls.is_empty() {
                    let tcs: Vec<serde_json::Value> = rr
                        .tool_calls
                        .iter()
                        .map(|tc| {
                            json!({
                                "id": tc.id,
                                "type": "function",
                                "function": { "name": tc.name, "arguments": tc.arguments },
                            })
                        })
                        .collect();
                    asst_obj.insert("tool_calls".into(), json!(tcs));
                }
                sub_msgs.push(serde_json::Value::Object(asst_obj));

                // 发射子 Agent 步骤事件（通知前端）
                let step_content = if rr.content.is_empty() {
                    "[工具调用]".to_string()
                } else {
                    truncate_for_display(&rr.content, 300)
                };
                let _ = window.emit(
                    "subagent-step",
                    SubagentStepPayload {
                        parent_assistant_id: parent_assistant_id.to_string(),
                        parent_topic_id: parent_topic_id.to_string(),
                        subagent_id: subagent_id.clone(),
                        round: round as u32,
                        step_type: if rr.tool_calls.is_empty() {
                            "content".to_string()
                        } else {
                            "tool_call".to_string()
                        },
                        summary: step_content,
                    },
                );

                // 输出被截断（finish_reason="length"）且无工具调用：追加续写指令后重试一次
                if rr.finish_reason.as_deref() == Some("length")
                    && rr.tool_calls.is_empty()
                    && length_retries < 1
                {
                    length_retries += 1;
                    sub_msgs.push(json!({
                        "role": "system",
                        "content": "[输出被截断] 请直接继续完成上一条回复中未完成的内容，不要重复已经输出的部分。"
                    }));
                    continue;
                }

                // 无工具调用 → 子 Agent 完成
                if rr.tool_calls.is_empty() {
                    final_text = rr.content;
                    break;
                }

                // 执行工具调用（内联，不通过 execute_builtin_tool，因为子 Agent 的工具受限）
                for tc in &rr.tool_calls {
                    if token.is_cancelled() {
                        break;
                    }
                    let args_val: serde_json::Value =
                        serde_json::from_str(&tc.arguments).unwrap_or(json!({}));

                    // 再次确认工具被允许（双重保险）
                    if !profile.is_tool_allowed(&tc.name) {
                        let err_text = format!(
                            "[已阻止] 子 Agent ({}) 不允许使用工具: {}",
                            profile.id, tc.name
                        );
                        sub_msgs.push(json!({
                            "role": "tool",
                            "content": err_text,
                            "tool_call_id": tc.id,
                            "name": tc.name,
                        }));
                        continue;
                    }

                    let tool_result = execute_builtin_tool(
                        app, &tc.name, &args_val, project_id,
                        agent_mode, // AGT-03：子 Agent 继承父 agent_mode，写/删/执行按父模式走审批
                        token,
                    )
                    .await;

                    let (content_text, _result_value, _is_error) = match tool_result {
                        Ok(tr) => {
                            let text = tr
                                .content
                                .iter()
                                .find(|c| c.kind == "text")
                                .and_then(|c| c.data.get("text"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| {
                                    serde_json::to_string(&tr.content).unwrap_or_default()
                                });
                            let result_value =
                                serde_json::to_value(&tr.content).unwrap_or(json!([]));
                            (text, result_value, tr.is_error)
                        }
                        Err(e) => (format!("[Error] {}", e), json!({ "error": e }), true),
                    };

                    // 发射工具执行结果事件
                    let _ = window.emit(
                        "subagent-step",
                        SubagentStepPayload {
                            parent_assistant_id: parent_assistant_id.to_string(),
                            parent_topic_id: parent_topic_id.to_string(),
                            subagent_id: subagent_id.clone(),
                            round: round as u32,
                            step_type: format!("tool_result:{}", tc.name),
                            summary: truncate_for_display(&content_text, 200),
                        },
                    );

                    // 回填 tool 消息：空结果占位 + 截断防上下文膨胀（与主循环一致）
                    let truncated_content = if content_text.trim().is_empty() {
                        "[无结果]".to_string()
                    } else {
                        truncate_tool_result(&content_text)
                    };
                    sub_msgs.push(json!({
                        "role": "tool",
                        "content": truncated_content,
                        "tool_call_id": tc.id,
                        "name": tc.name,
                    }));
                }

                // 子 Agent 上下文安全阀：估算 token 超保守预算时压缩（无需模型窗口信息，
                // 对绝大多数 ≥64k 窗口的模型都安全；压缩保留最近 token 预算）
                let sub_est: usize =
                    estimate_context_tokens(model_to_use, &sub_msgs, SUBAGENT_CONTEXT_BUDGET);
                if sub_est > SUBAGENT_CONTEXT_BUDGET && sub_msgs.len() > 4 {
                    let keep = (SUBAGENT_CONTEXT_BUDGET / 2).clamp(4_000, 20_000);
                    let mut msgs = std::mem::take(&mut sub_msgs);
                    let _ = compress_context(
                        window,
                        client,
                        api_url,
                        api_key,
                        model_to_use,
                        &mut msgs,
                        parent_assistant_id,
                        parent_topic_id,
                        token,
                        keep,
                        "subagent",
                    )
                    .await;
                    sub_msgs = msgs;
                }
            }
            Err(e) => {
                if e == "cancelled" {
                    break;
                }
                // 上下文溢出自动恢复：压缩后重试一次
                if sub_overflow_retries < 1 && is_context_overflow_error(&e) && sub_msgs.len() > 4 {
                    sub_overflow_retries += 1;
                    let keep = (SUBAGENT_CONTEXT_BUDGET / 2).clamp(4_000, 20_000);
                    let mut msgs = std::mem::take(&mut sub_msgs);
                    let res = compress_context(
                        window,
                        client,
                        api_url,
                        api_key,
                        model_to_use,
                        &mut msgs,
                        parent_assistant_id,
                        parent_topic_id,
                        token,
                        keep,
                        "subagent",
                    )
                    .await;
                    sub_msgs = msgs;
                    if res.is_ok() {
                        tracing::warn!("[subagent overflow] 上下文溢出，已压缩并重试: {}", e);
                        continue;
                    }
                }
                // API 错误：记录并返回
                let _ = window.emit(
                    "subagent-error",
                    SubagentErrorPayload {
                        parent_assistant_id: parent_assistant_id.to_string(),
                        parent_topic_id: parent_topic_id.to_string(),
                        subagent_id: subagent_id.clone(),
                        error: e.clone(),
                    },
                );
                return Err(format!("子 Agent API 错误: {}", e));
            }
        }
    }

    // 子 Agent 完成：发射 done 事件
    let result_text = if final_text.is_empty() {
        // 无最终文本时回退为已完成工作摘要，保证主 Agent 始终拿到有效返回
        build_subagent_progress_summary(&sub_msgs, MAX_SUBAGENT_ROUNDS)
    } else {
        final_text
    };

    let _ = window.emit(
        "subagent-done",
        SubagentDonePayload {
            parent_assistant_id: parent_assistant_id.to_string(),
            parent_topic_id: parent_topic_id.to_string(),
            subagent_id: subagent_id.clone(),
            profile_name: profile.name.clone(),
            result: result_text.clone(),
        },
    );

    Ok(ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": format!(
                "[子智能体「{}」工作报告]\n\n{}",
                profile.name,
                result_text,
            )}),
        }],
        is_error: false,
    })
}

// ===== 子 Agent 事件载荷定义 =====

#[derive(Serialize, Clone)]
struct SubagentStartPayload {
    parent_assistant_id: String,
    parent_topic_id: String,
    subagent_id: String,
    profile_id: String,
    profile_name: String,
    task_summary: String,
}

#[derive(Serialize, Clone)]
struct SubagentStepPayload {
    parent_assistant_id: String,
    parent_topic_id: String,
    subagent_id: String,
    round: u32,
    step_type: String,
    summary: String,
}

#[derive(Serialize, Clone)]
struct SubagentDonePayload {
    parent_assistant_id: String,
    parent_topic_id: String,
    subagent_id: String,
    profile_name: String,
    result: String,
}

#[derive(Serialize, Clone)]
struct SubagentErrorPayload {
    parent_assistant_id: String,
    parent_topic_id: String,
    subagent_id: String,
    error: String,
}

/// Agent 自驱循环命令（前端新流程主入口）。
///
/// 在单个 tokio 任务内完成「流式 → 检测工具 → 权限/审批 → 执行 MCP → 回填结果 → 递归」，
/// 直到模型不再发起工具调用或达到轮数上限。前端只需监听事件做纯渲染：
/// - `llm-round-start`：push 空 assistant 占位消息
/// - `llm-chunk`：追加文本 / `done:true` 表示整轮真正结束
/// - `llm-reasoning`：追加思维链
/// - `llm-tool-call`：展示"调用中"气泡（仅通知，执行由本命令完成）
/// - `llm-tool-result`：更新气泡状态 + 追加 role:tool 消息
/// - `tool-approval-requested`：审批请求
///
/// # 参数
/// - `messages`：初始消息列表（含本轮 user 消息），由前端构造好 system/历史/user
/// - `mcp_server_ids`：保留参数（主 Agent 门控后不再注入 MCP 工具，子 Agent 自建工具集）
/// - `agent_mode`：Agent 执行模式，影响权限规则与工具注入（Plan: 研究工具 / Normal/Auto/Workflow: 编排工具 / Off: 无工具）
/// - `project_id`：项目 id（用于解析项目级权限规则）
/// - `locale`：可选界面语言；旧调用缺失时兼容回退到 `zh-CN`
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri IPC compatibility requires the existing flat agent parameters"
)]
pub async fn run_agent_turn(
    app: AppHandle,
    window: Window,
    db_state: tauri::State<'_, DbState>,
    stream_mgr: tauri::State<'_, StreamManager>,
    api_url: String,
    api_key: String,
    model: String,
    assistant_id: String,
    topic_id: String,
    messages: Vec<Message>,
    _mcp_server_ids: Vec<String>,
    agent_mode: AgentMode,
    project_id: Option<String>,
    web_search_enabled: bool,
    profile_model_overrides: Vec<ProfileModelOverride>,
    custom_subagent_profiles: Vec<crate::core::models::CustomSubagentProfile>,
    locale: Option<String>,
    context_window: Option<u32>,
    skills: Option<Vec<SkillConfig>>,
) -> Result<(), String> {
    // 诊断日志：打印 messages 每个元素的 role 和关键字段是否存在
    for (i, m) in messages.iter().enumerate() {
        tracing::info!(
            "[run_agent_turn] messages[{}] role={} has_toolCallId={} has_toolCalls={} has_name={}",
            i,
            m.role,
            m.tool_call_id.is_some(),
            m.tool_calls.is_some(),
            m.name.is_some()
        );
    }
    // SSRF 防护
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    let task_key = format!("{}-{}", assistant_id, topic_id);

    // 取消同 topic 的旧任务（cancel 而非 abort）。
    // 必须 await 旧 handle 完成 epilogue，否则旧任务的 state_inner.remove
    // 可能在新的 insert 之后执行，误删新任务的条目导致新任务变成孤儿。
    if let Some((_, (old_handle, old_token))) = stream_mgr.0.remove(&task_key) {
        old_token.cancel();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(10), old_handle).await;
    }

    // 预先把 messages 转为 API 格式（含附件 image 展开等），在持锁期间完成同步 I/O
    let mut messages_for_api: Vec<serde_json::Value> = {
        let conn = db_state.0.lock();
        messages
            .iter()
            .map(|m| message_for_api(&conn, m))
            .collect::<Result<Vec<_>, _>>()?
    };
    verify_tool_messages(&messages_for_api);
    let locale_c = if locale.as_deref() == Some("en-US") {
        "en-US".to_string()
    } else {
        "zh-CN".to_string()
    };
    messages_for_api.insert(
        0,
        json!({
            "role": "system",
            "content": if locale_c == "en-US" {
                "Use English for AIO-controlled explanations, plans, and summaries. Preserve user content, code, identifiers, tool names, paths, and external output verbatim."
            } else {
                "AIO 控制的说明、计划和总结请使用简体中文。用户内容、代码、标识符、工具名、路径和外部输出保持原文。"
            }
        }),
    );

    let is_agent_mode = agent_mode != AgentMode::Off;

    // ===== Token 效率：模型感知的上下文预算 =====
    // 基于模型上下文窗口（前端从模型目录传入，缺省 128k）推导压缩触发阈值，
    // 参考 oh-my-pi 的 reserve 策略：reserve = max(16384, 15% 窗口)。
    let context_window_n = context_window.unwrap_or(128_000).max(16_000) as usize;
    let context_reserve = 16_384usize.max(context_window_n / 100 * 15);
    let context_budget = context_window_n.saturating_sub(context_reserve); // 触发压缩的 token 阈值
    let keep_recent_tokens = (context_budget / 2).clamp(4_000, 20_000); // 压缩时保留的最近 token 预算

    // 推理类模型（OpenAI o1/o3/o4、gpt-5 系列）不接受 max_tokens 字段（需 max_completion_tokens），
    // 对其不注入输出上限避免 API 报错；其余模型注入窗口一半的输出预算。
    let model_lower = model.to_lowercase();
    let is_reasoning_model = ["o1", "o3", "o4", "gpt-5"]
        .iter()
        .any(|p| model_lower.contains(p));
    let max_output_tokens = if is_reasoning_model {
        None
    } else {
        Some(((context_window_n / 2) as u32).clamp(1024, 16_384))
    };

    let token = CancellationToken::new();
    let token_inner = token.clone();
    let state_inner = stream_mgr.0.clone();
    let task_key_inner = task_key.clone();
    let assistant_id_c = assistant_id.clone();
    let topic_id_c = topic_id.clone();
    let app_c = app.clone();
    let project_id_c = project_id.clone();
    let profile_overrides_c = profile_model_overrides.clone();
    let custom_profiles_c = custom_subagent_profiles.clone();

    let handle = tokio::spawn(async move {
        // 解包 Skills（前端仅在 Agent 模式下传递；聊天模式为空）
        let skills = skills.unwrap_or_default();
        // 加载应用配置（含自动重试设置）
        let app_config =
            crate::commands::config::load_app_config(app_c.clone()).unwrap_or_else(|_| AppConfig {
                api_url: String::new(),
                api_key: String::new(),
                default_model: String::new(),
                local_model_path: String::new(),
                auto_retry_enabled: true,
                auto_retry_count: 2,
                auto_retry_delay_ms: 500,
                knowledge_enabled: false,
                auto_start_enabled: false,
                max_concurrent_subagents: None,
                max_tool_rounds: None,
                memory_enabled: false,
                memory_auto_inject: true,
                memory_auto_extract: true,
                memory_max_facts: 2000,
                memory_injection_budget_tokens: 3000,
                memory_extract_debounce_secs: 60,
                memory_embedding: crate::core::models::MemoryEmbeddingConfig::default(),
            });

        // 子 Agent 并发上限控制
        let max_concurrent = app_config.max_concurrent_subagents.unwrap_or(5).max(1) as usize;
        let subagent_semaphore = Arc::new(Semaphore::new(max_concurrent));

        let client = streaming_http_client();

        // 工具选择：根据 Agent 模式注入不同工具集
        // - Plan: 仅研究工具（think + project_map + web）
        // - Normal/Auto/Workflow: 仅编排工具（delegate_task + create_workflow + think + web + project_map）
        // - 对话 + 联网: 仅 web 工具
        // - 纯对话: 无工具
        let (mut tools, mut tool_server_map) = if agent_mode == AgentMode::Plan {
            // Plan 模式：仅研究工具（think + project_map + web）
            let mut plan_tools: Vec<ToolSpec> = Vec::new();
            let mut plan_map: HashMap<String, String> = HashMap::new();

            // think
            let think_spec = crate::utils::think::tool_spec();
            plan_map.insert(think_spec.function.name.clone(), "__builtin__".into());
            plan_tools.push(think_spec);

            // project_map
            let pm_spec = crate::utils::project_map::tool_spec();
            plan_map.insert(pm_spec.function.name.clone(), "__builtin__".into());
            plan_tools.push(pm_spec);

            // web tools
            for spec in web_tools::get_web_tool_specs() {
                plan_map.insert(spec.function.name.clone(), "__builtin__".into());
                plan_tools.push(spec);
            }

            (plan_tools, plan_map)
        } else if is_agent_mode {
            // Normal/Auto/Workflow: 仅编排工具
            let mut orch_tools: Vec<ToolSpec> = Vec::new();
            let mut orch_map: HashMap<String, String> = HashMap::new();

            // delegate_task
            let delegate_spec = subagent::delegate_task_tool_spec();
            orch_map.insert(delegate_spec.function.name.clone(), "__builtin__".into());
            orch_tools.push(delegate_spec);

            // delegate_tasks（批量并行）
            let delegate_batch_spec = subagent::delegate_tasks_tool_spec();
            orch_map.insert(
                delegate_batch_spec.function.name.clone(),
                "__builtin__".into(),
            );
            orch_tools.push(delegate_batch_spec);

            // create_workflow
            let workflow_spec = subagent::create_workflow_tool_spec();
            orch_map.insert(workflow_spec.function.name.clone(), "__builtin__".into());
            orch_tools.push(workflow_spec);

            // think
            let think_spec = crate::utils::think::tool_spec();
            orch_map.insert(think_spec.function.name.clone(), "__builtin__".into());
            orch_tools.push(think_spec);

            // web tools
            for spec in web_tools::get_web_tool_specs() {
                orch_map.insert(spec.function.name.clone(), "__builtin__".into());
                orch_tools.push(spec);
            }

            // project_map
            let pm_spec = crate::utils::project_map::tool_spec();
            orch_map.insert(pm_spec.function.name.clone(), "__builtin__".into());
            orch_tools.push(pm_spec);

            (orch_tools, orch_map)
        } else if web_search_enabled {
            // 仅注入 Web 工具（对话模式下联网搜索）
            let web_specs = web_tools::get_web_tool_specs();
            let mut mcp_map = std::collections::HashMap::new();
            for spec in &web_specs {
                mcp_map.insert(spec.function.name.clone(), "__builtin__".into());
            }
            (web_specs, mcp_map)
        } else {
            (Vec::new(), std::collections::HashMap::new())
        };
        // 记忆工具：全局默认 + 每项目覆盖决定是否启用；启用时注入记忆工具，否则回退旧知识工具
        let mut memory_effective = false;
        if is_agent_mode && project_id_c.is_some() {
            let root = file_tools::resolve_project_root(&app_c, project_id_c.as_deref()).ok();
            let override_enabled = root
                .and_then(|r| {
                    app_c
                        .state::<crate::services::memory::MemoryStoreManager>()
                        .open(&r)
                        .ok()
                })
                .and_then(|s| s.project_enabled().ok())
                .flatten();
            memory_effective = override_enabled.unwrap_or(app_config.memory_enabled);
            if memory_effective {
                for spec in crate::services::memory::tools::get_memory_tool_specs() {
                    tool_server_map.insert(spec.function.name.clone(), "__builtin__".into());
                    tools.push(spec);
                }
            } else if app_config.knowledge_enabled {
                for spec in knowledge::get_knowledge_tool_specs() {
                    tool_server_map.insert(spec.function.name.clone(), "__builtin__".into());
                    tools.push(spec);
                }
            }
        }
        // P1-10：Skills 按需注入 — 仅注入 read_skill 工具，由模型按需读取完整内容
        if is_agent_mode && !skills.is_empty() {
            let spec = skill_read_tool_spec();
            tool_server_map.insert(spec.function.name.clone(), "__builtin__".into());
            tools.push(spec);
        }
        // 历史归档读取工具（压缩回退归档的恢复通道）
        if is_agent_mode {
            let spec = artifact_read_tool_spec();
            tool_server_map.insert(spec.function.name.clone(), "__builtin__".into());
            tools.push(spec);
        }
        let tools_slice: Option<&[ToolSpec]> = if tools.is_empty() { None } else { Some(&tools) };

        let mut round: u32 = 0;
        let mut final_error: Option<String> = None;
        let mut was_cancelled = false;
        // Workflow 模式：标记是否已调用 create_workflow
        let mut workflow_called = false;
        // 跨轮累计 token 用量（用于成本统计）
        let mut total_input_tokens: u32 = 0;
        let mut total_output_tokens: u32 = 0;
        let mut total_cached_tokens: u32 = 0;
        // 前缀稳定性回归验证：标记本轮刚发生压缩，用于记录压缩后首轮的缓存命中情况
        let mut compressed_this_run: bool = false;
        // 未完成任务跟踪（handoff 交接文档）：强制停止（轮数上限/工作流未调用）或错误
        let mut incomplete_stop: bool = false;
        let mut last_finish_reason: Option<String> = None;
        // 上下文峰值 tokens：最后一轮 API 调用的 input_tokens（用于上下文窗口展示）
        let mut context_input_tokens: u32 = 0;
        // 跨轮累计的最终文本与生成图像（用于 done 时一次性回传，幂等替换前端 chunk 累积）
        let mut accumulated_content = String::new();
        let mut all_images: Vec<GeneratedImage> = Vec::new();

        // Workflow 模式：在 LLM 循环前注入强制系统消息（置于稳定前缀内，利于 prompt cache 与压缩保护）
        if agent_mode == AgentMode::Workflow {
            messages_for_api.insert(
                2.min(messages_for_api.len()),
                serde_json::json!({
                    "role": "system",
                    "content": concat!(
                        "你正处于「工作流模式」。首要任务：分析用户请求 → 调用 `create_workflow` 创建并执行工作流。\n",
                        "不得直接修改文件、执行命令或调用其他工具。\n",
                        "典型序列：requirements → coder → reviewer；explorer → coder；debugger → coder；requirements → architect → coder → tester。\n",
                        "简单请求可用单步工作流。请立即调用 create_workflow 开始。"
                    )
                }),
            );
        }

        // 注入项目知识到系统提示词（跨 session 记忆）—— 仅当用户在设置中开启时
        if app_config.knowledge_enabled {
            if let Some(pid) = &project_id_c {
                if let Ok(project_root) = file_tools::resolve_project_root(&app_c, Some(pid)) {
                    let project_knowledge = knowledge::load_knowledge(&project_root);
                    if !project_knowledge.entries.is_empty() {
                        let prompt = knowledge::knowledge_to_prompt(&project_knowledge);
                        messages_for_api.insert(
                            1,
                            serde_json::json!({
                                "role": "system",
                                "content": format!("[项目知识 — 来自之前对话]\n{}", prompt)
                            }),
                        );
                    }
                }
            }
        }
        // 项目记忆自动注入（P1）：基于首条 user 消息检索一次，插入稳定前缀位（index=1）
        if memory_effective && app_config.memory_auto_inject {
            if let Some(pid) = &project_id_c {
                if let Ok(project_root) = file_tools::resolve_project_root(&app_c, Some(pid)) {
                    if let Ok(store) = app_c
                        .state::<crate::services::memory::MemoryStoreManager>()
                        .open(&project_root)
                    {
                        let query = messages_for_api
                            .iter()
                            .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
                            .and_then(|m| m.get("content").and_then(|v| v.as_str()))
                            .unwrap_or("")
                            .to_string();
                        let budget = app_config.memory_injection_budget_tokens as usize;
                        if !query.is_empty() && budget > 0 {
                            let mut ecfg = app_config.memory_embedding.clone();
                            if ecfg.provider == "openai_compat" && ecfg.api_key.is_empty() {
                                if let Ok(Some(key)) =
                                    crate::core::secure_store::get(&app_c, "embedding_api_key")
                                {
                                    ecfg.api_key = key;
                                }
                            }
                            let query_vec = match crate::plugins::embed::resolve(&ecfg) {
                                Ok(e) => e
                                    .embed(std::slice::from_ref(&query))
                                    .await
                                    .ok()
                                    .and_then(|v| v.into_iter().next()),
                                Err(_) => None,
                            };
                            if let Ok(Some(block)) =
                                store.build_inject_block(&query, query_vec.as_deref(), budget)
                            {
                                messages_for_api.insert(
                                    1,
                                    serde_json::json!({
                                        "role": "system",
                                        "content": format!("[项目记忆]\n{}", block)
                                    }),
                                );
                            }
                        }
                    }
                }
            }
        }
        // 上下文预算追踪：token 级、模型感知（阈值/保留预算在任务启动前按窗口推导）
        // AGT-01：可配置的单次最大工具调用轮数（默认 25，最低 1）
        let max_rounds = app_config
            .max_tool_rounds
            .unwrap_or(crate::core::models::DEFAULT_MAX_TOOL_ROUNDS)
            .max(1);
        // AGT-02：Workflow 模式下提醒调用 create_workflow 的最大次数（有限次，避免无限循环）
        let max_workflow_reminders: u32 = 2;
        let mut workflow_reminders: u32 = 0;
        // 输出截断（finish_reason="length"）时的最大续写重试次数
        const MAX_LENGTH_RETRIES: u32 = 2;
        let mut length_retries: u32 = 0;
        // 上下文溢出（context overflow）自动恢复次数：压缩后重试
        const MAX_OVERFLOW_RETRIES: u32 = 1;
        let mut overflow_retries: u32 = 0;

        'outer: loop {
            round += 1;
            if token_inner.is_cancelled() {
                was_cancelled = true;
                break;
            }

            // AGT-01：达到最大工具调用轮数 → 优雅收尾（不 abort，产出提示后自然结束）
            if round > max_rounds {
                incomplete_stop = true;
                let note = "\n\n⚠️ 已达本轮最大工具调用轮数，任务未完成。如需继续，请再次发送。";
                accumulated_content.push_str(note);
                let _ = window.emit(
                    "llm-chunk",
                    StreamPayload {
                        assistant_id: assistant_id_c.clone(),
                        topic_id: topic_id_c.clone(),
                        content: note.into(),
                        done: false,
                        error: None,
                        input_tokens: None,
                        output_tokens: None,
                        context_tokens: None,
                        cached_input_tokens: None,
                        images: None,
                    },
                );
                break 'outer;
            }

            // 通知前端：新一轮开始，push 空 assistant 占位
            let _ = window.emit(
                "llm-round-start",
                RoundStartPayload {
                    assistant_id: assistant_id_c.clone(),
                    topic_id: topic_id_c.clone(),
                    round,
                },
            );

            // 不设轮数硬上限：复杂任务可能需要多轮工具调用，循环仅在模型不再发起工具调用
            // （任务完成）或用户点停止（token 取消）时自然结束。
            let (round_msgs, round_tools) = (messages_for_api.as_slice(), tools_slice);

            let round_result = stream_one_round(
                &window,
                &token_inner,
                &client,
                api_url.clone(),
                &api_key,
                &model,
                round_msgs,
                round_tools,
                &assistant_id_c,
                &topic_id_c,
                false,
                Some(&app_c),
                true,
                max_output_tokens,
            )
            .await;

            let round_result = match round_result {
                Ok(r) => r,
                Err(e) => {
                    if e == "cancelled" {
                        was_cancelled = true;
                        break 'outer;
                    }
                    // 上下文溢出自动恢复：压缩早期消息后重试一次，避免长会话直接失败
                    if overflow_retries < MAX_OVERFLOW_RETRIES
                        && is_context_overflow_error(&e)
                        && messages_for_api.len() > 4
                    {
                        overflow_retries += 1;
                        let mut msgs = std::mem::take(&mut messages_for_api);
                        let res = compress_context(
                            &window,
                            &client,
                            &api_url,
                            &api_key,
                            &model,
                            &mut msgs,
                            &assistant_id_c,
                            &topic_id_c,
                            &token_inner,
                            keep_recent_tokens,
                            "main",
                        )
                        .await;
                        messages_for_api = msgs;
                        if res.is_ok() {
                            tracing::warn!("[overflow] 上下文溢出，已压缩并重试: {}", e);
                            continue;
                        }
                    }
                    incomplete_stop = true;
                    final_error = Some(e);
                    break 'outer;
                }
            };

            // 累计 token 用量
            total_input_tokens += round_result.input_tokens;
            total_output_tokens += round_result.output_tokens;
            total_cached_tokens += round_result.cached_input_tokens;
            last_finish_reason = round_result.finish_reason.clone();

            // 前缀稳定性回归验证：压缩后首轮的缓存命中率（稳定前缀块应继续命中）
            if compressed_this_run {
                compressed_this_run = false;
                let hit_pct = if round_result.input_tokens > 0 {
                    (round_result.cached_input_tokens as f64 / round_result.input_tokens as f64)
                        * 100.0
                } else {
                    0.0
                };
                tracing::info!(
                    "[cache] 压缩后首轮 input_tokens={}, cached_input_tokens={}（命中率 {:.1}%）",
                    round_result.input_tokens,
                    round_result.cached_input_tokens,
                    hit_pct
                );
            }
            // 记录峰值上下文（最后一轮的 input_tokens）
            context_input_tokens = round_result.input_tokens;
            // 累计最终文本与生成图像（含 aio-image 标记，供 done 回传）
            accumulated_content.push_str(&round_result.content);
            all_images.extend(round_result.images.clone());

            // 持久化本轮 token 用量到 usage_log
            insert_usage_log(
                &app_c,
                &assistant_id_c,
                &topic_id_c,
                &model,
                round,
                round_result.input_tokens,
                round_result.output_tokens,
                round_result.cached_input_tokens,
            );

            // 把本轮 assistant 消息（含 tool_calls）append 到上下文
            let mut asst_obj = serde_json::Map::new();
            asst_obj.insert("role".into(), json!("assistant"));
            asst_obj.insert(
                "content".into(),
                match &round_result.api_content {
                    serde_json::Value::String(s) if s.is_empty() => json!(null),
                    other => other.clone(),
                },
            );
            if !round_result.tool_calls.is_empty() {
                let tcs: Vec<serde_json::Value> = round_result
                    .tool_calls
                    .iter()
                    .map(|tc| {
                        json!({
                            "id": tc.id,
                            "type": "function",
                            "function": { "name": tc.name, "arguments": tc.arguments },
                        })
                    })
                    .collect();
                asst_obj.insert("tool_calls".into(), json!(tcs));
            }
            messages_for_api.push(serde_json::Value::Object(asst_obj));

            // 输出被截断（finish_reason="length"）且无工具调用：追加续写指令后重试（有限次），
            // 避免半截回复直接结束导致用户重发整条消息浪费 token。
            if round_result.finish_reason.as_deref() == Some("length")
                && round_result.tool_calls.is_empty()
                && length_retries < MAX_LENGTH_RETRIES
            {
                length_retries += 1;
                messages_for_api.push(serde_json::json!({
                    "role": "system",
                    "content": "[输出被截断] 请直接继续完成上一条回复中未完成的内容，不要重复已经输出的部分。"
                }));
                continue;
            }

            // 无工具调用 → 任务完成，整轮结束
            if round_result.tool_calls.is_empty() {
                if agent_mode == AgentMode::Workflow && !workflow_called {
                    // Workflow 模式：提醒次数有限（max_workflow_reminders），避免无限续环（AGT-02）
                    if workflow_reminders < max_workflow_reminders {
                        workflow_reminders += 1;
                        messages_for_api.push(serde_json::json!({
                            "role": "system",
                            "content": "【工作流模式强制指令】你尚未调用 create_workflow 工具。请立即分析用户请求并调用 create_workflow 来创建和执行工作流。"
                        }));
                        continue;
                    }
                    // 有限次提醒后模型仍拒绝 → 尊重退出信号，优雅收尾
                    incomplete_stop = true;
                    let note = "\n\n⚠️ 模型未按工作流模式调用 create_workflow，已停止。您可以切回普通/自动模式继续。";
                    accumulated_content.push_str(note);
                    let _ = window.emit(
                        "llm-chunk",
                        StreamPayload {
                            assistant_id: assistant_id_c.clone(),
                            topic_id: topic_id_c.clone(),
                            content: note.into(),
                            done: false,
                            error: None,
                            input_tokens: None,
                            output_tokens: None,
                            context_tokens: None,
                            cached_input_tokens: None,
                            images: None,
                        },
                    );
                    break 'outer;
                }
                break 'outer;
            }

            // 执行工具调用：两阶段
            // 阶段 1 — 收集 delegate_task futures + 顺序执行非 delegate 工具
            // 阶段 2 — 并发等待所有子 Agent，然后按原顺序发射结果

            // 用于记录每个工具的结果（按原始 index 排序）
            #[derive(Default, Clone)]
            struct ToolExecResult {
                content_text: String,
                result_value: serde_json::Value,
                is_error: bool,
            }
            let mut exec_results: Vec<Option<ToolExecResult>> =
                vec![None; round_result.tool_calls.len()];

            // 收集 delegate_task 的 future 和对应的 index
            let mut delegate_futures: Vec<(
                usize,
                futures_util::future::BoxFuture<'static, Result<ToolResult, String>>,
            )> = Vec::new();

            for (i, tc) in round_result.tool_calls.iter().enumerate() {
                if token_inner.is_cancelled() {
                    was_cancelled = true;
                    break;
                }
                let server_id = tool_server_map.get(&tc.name).cloned();
                let args_val: serde_json::Value =
                    serde_json::from_str(&tc.arguments).unwrap_or(json!({}));

                if server_id.as_deref() == Some("__builtin__") && tc.name == "delegate_task" {
                    // 检查 wait 参数：false = 后台执行（fire-and-forget）
                    let wait_for_result = args_val
                        .get("wait")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    if !wait_for_result {
                        // Fire-and-forget: 子智能体在后台运行，主 Agent 立即继续
                        let window_clone = window.clone();
                        let app_clone = app_c.clone();
                        let client_clone = client.clone();
                        let api_url_clone = api_url.clone();
                        let api_key_clone = api_key.clone();
                        let model_clone = model.clone();
                        let assistant_id_clone = assistant_id_c.clone();
                        let topic_id_clone = topic_id_c.clone();
                        let args_val_clone = args_val.clone();
                        let project_id_clone = project_id_c.clone();
                        let agent_mode_clone = agent_mode.clone();
                        let profile_overrides_clone = profile_overrides_c.clone();
                        let custom_profiles_clone = custom_profiles_c.clone();
                        let token_clone = token_inner.clone();
                        let locale_clone = locale_c.clone();
                        let sem_clone = subagent_semaphore.clone();
                        let subagent_id = uuid::Uuid::new_v4().to_string();
                        let sid = subagent_id.clone();
                        let app_c2 = app_c.clone();
                        tokio::spawn(async move {
                            let result = handle_delegate_task(
                                &window_clone,
                                &app_clone,
                                &client_clone,
                                &api_url_clone,
                                &api_key_clone,
                                &model_clone,
                                &assistant_id_clone,
                                &topic_id_clone,
                                &args_val_clone,
                                project_id_clone.as_deref(),
                                &agent_mode_clone,
                                sem_clone,
                                &token_clone,
                                &profile_overrides_clone,
                                &custom_profiles_clone,
                                &locale_clone,
                            )
                            .await;
                            // Remove handle from registry on completion
                            if let Some(handles) =
                                app_c2.try_state::<crate::core::state::SubagentHandles>()
                            {
                                handles.0.remove(&sid);
                            }
                            let _ = result;
                        });
                        // Immediate result: acknowledge the fire-and-forget
                        let content_text = format!("子智能体已在后台启动 (ID: {})", subagent_id);
                        exec_results[i] = Some(ToolExecResult {
                            content_text: content_text.clone(),
                            result_value: json!({"status": "started", "subagent_id": subagent_id}),
                            is_error: false,
                        });
                    } else {
                        // 子智能体委托：收集 future，稍后并行执行
                        let window_clone = window.clone();
                        let app_clone = app_c.clone();
                        let client_clone = client.clone();
                        let api_url_clone = api_url.clone();
                        let api_key_clone = api_key.clone();
                        let model_clone = model.clone();
                        let assistant_id_clone = assistant_id_c.clone();
                        let topic_id_clone = topic_id_c.clone();
                        let args_val_clone = args_val.clone();
                        let project_id_clone = project_id_c.clone();
                        let agent_mode_clone = agent_mode.clone();
                        let profile_overrides_clone = profile_overrides_c.clone();
                        let custom_profiles_clone = custom_profiles_c.clone();
                        let token_clone = token_inner.clone();
                        let locale_clone = locale_c.clone();
                        let sem_clone = subagent_semaphore.clone();

                        let fut = Box::pin(async move {
                            handle_delegate_task(
                                &window_clone,
                                &app_clone,
                                &client_clone,
                                &api_url_clone,
                                &api_key_clone,
                                &model_clone,
                                &assistant_id_clone,
                                &topic_id_clone,
                                &args_val_clone,
                                project_id_clone.as_deref(),
                                &agent_mode_clone,
                                sem_clone,
                                &token_clone,
                                &profile_overrides_clone,
                                &custom_profiles_clone,
                                &locale_clone,
                            )
                            .await
                        });
                        delegate_futures.push((i, fut));
                    }
                } else if server_id.as_deref() == Some("__builtin__") && tc.name == "delegate_tasks"
                {
                    // 批量并行：收集一个 future，放入 delegate_futures
                    let window_clone = window.clone();
                    let app_clone = app_c.clone();
                    let client_clone = client.clone();
                    let api_url_clone = api_url.clone();
                    let api_key_clone = api_key.clone();
                    let model_clone = model.clone();
                    let assistant_id_clone = assistant_id_c.clone();
                    let topic_id_clone = topic_id_c.clone();
                    let args_val_clone = args_val.clone();
                    let project_id_clone = project_id_c.clone();
                    let agent_mode_clone = agent_mode.clone();
                    let profile_overrides_clone = profile_overrides_c.clone();
                    let custom_profiles_clone = custom_profiles_c.clone();
                    let semaphore_clone = subagent_semaphore.clone();
                    let token_clone = token_inner.clone();
                    let locale_clone = locale_c.clone();

                    let fut = Box::pin(async move {
                        handle_delegate_tasks(
                            &window_clone,
                            &app_clone,
                            &client_clone,
                            &api_url_clone,
                            &api_key_clone,
                            &model_clone,
                            &assistant_id_clone,
                            &topic_id_clone,
                            &args_val_clone,
                            project_id_clone.as_deref(),
                            &agent_mode_clone,
                            &token_clone,
                            &profile_overrides_clone,
                            &custom_profiles_clone,
                            semaphore_clone,
                            &locale_clone,
                        )
                        .await
                    });
                    delegate_futures.push((i, fut));
                } else if server_id.as_deref() == Some("__builtin__")
                    && tc.name == "create_workflow"
                {
                    // 工作流创建：解析参数并顺序执行所有步骤
                    workflow_called = true;
                    let title = args_val["title"].as_str().unwrap_or("Workflow").to_string();
                    let steps_raw = args_val["steps"].as_array();
                    let tool_result = if let Some(steps_arr) = steps_raw {
                        if steps_arr.is_empty() {
                            Err("create_workflow: 'steps' 数组不能为空".to_string())
                        } else {
                            let mut workflow_steps: Vec<WorkflowStep> = Vec::new();
                            let mut parse_error: Option<String> = None;
                            for (idx, step) in steps_arr.iter().enumerate() {
                                let profile =
                                    step["profile"].as_str().unwrap_or("general").to_string();
                                let name = step["name"].as_str().unwrap_or("").to_string();
                                let task = step["task"].as_str().unwrap_or("").to_string();
                                if task.is_empty() {
                                    parse_error = Some(format!(
                                        "create_workflow: 步骤 {} 缺少 'task' 字段",
                                        idx + 1
                                    ));
                                    break;
                                }
                                workflow_steps.push(WorkflowStep {
                                    step_id: format!("step-{}", idx + 1),
                                    profile_id: profile,
                                    name,
                                    task_description: task,
                                    status: WorkflowStepStatus::Pending,
                                    result: None,
                                    started_at: None,
                                    duration: None,
                                });
                            }
                            if let Some(err) = parse_error {
                                Err(err)
                            } else {
                                let workflow = Workflow {
                                    workflow_id: uuid::Uuid::new_v4().to_string(),
                                    title,
                                    steps: workflow_steps,
                                };
                                execute_workflow(
                                    &window,
                                    &app_c,
                                    &client,
                                    &api_url,
                                    &api_key,
                                    &model,
                                    &assistant_id_c,
                                    &topic_id_c,
                                    &workflow,
                                    project_id_c.as_deref(),
                                    subagent_semaphore.clone(),
                                    &custom_profiles_c,
                                    &profile_overrides_c,
                                    &token_inner,
                                    &locale_c,
                                )
                                .await
                            }
                        }
                    } else {
                        Err("create_workflow: 缺少 'steps' 数组".to_string())
                    };

                    let (content_text, result_value, is_error) = match tool_result {
                        Ok(tr) => {
                            let text = tr
                                .content
                                .iter()
                                .find(|c| c.kind == "text")
                                .and_then(|c| c.data.get("text"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| {
                                    serde_json::to_string(&tr.content).unwrap_or_default()
                                });
                            let result_value =
                                serde_json::to_value(&tr.content).unwrap_or(json!([]));
                            (text, result_value, tr.is_error)
                        }
                        Err(e) => (format!("[Error] {}", e), json!({ "error": e }), true),
                    };

                    exec_results[i] = Some(ToolExecResult {
                        content_text,
                        result_value,
                        is_error,
                    });
                } else {
                    // 非 delegate 工具：立即顺序执行
                    // 在工具执行前提取文件路径（args_val 可能被 move 到 MCP 调用中）
                    let file_path_for_diff: Option<String> =
                        if ["write_file", "replace_in_file", "delete_file"]
                            .contains(&tc.name.as_str())
                        {
                            args_val["path"].as_str().map(|s| s.to_string())
                        } else {
                            None
                        };
                    let tool_result = if server_id.as_deref() == Some("__builtin__")
                        && tc.name == "read_artifact"
                    {
                        execute_artifact_read(
                            &app_c,
                            args_val["artifact_id"].as_str().unwrap_or(""),
                        )
                    } else if server_id.as_deref() == Some("__builtin__") && tc.name == "read_skill"
                    {
                        execute_skill_read(&skills, args_val["name"].as_str().unwrap_or(""))
                    } else if server_id.as_deref() == Some("__builtin__") {
                        execute_builtin_tool(
                            &app_c,
                            &tc.name,
                            &args_val,
                            project_id_c.as_deref(),
                            &agent_mode,
                            &token_inner,
                        )
                        .await
                    } else {
                        // MCP 工具走自动重试包装器（网络错误可重试，权限/参数错误不重试）
                        let tc_name = tc.name.clone();
                        let sid_opt = server_id.clone();
                        let app_c2 = app_c.clone();
                        let args_val_c = args_val.clone();
                        let pid = project_id_c.clone();
                        let mode = agent_mode.clone();
                        let tok = token_inner.clone();
                        let ac = app_config.clone();
                        execute_tool_with_retry(
                            &tc_name,
                            || {
                                let app = app_c2.clone();
                                let args = args_val_c.clone();
                                let pid_c = pid.clone();
                                let mode_c = mode.clone();
                                let tok_c = tok.clone();
                                let sid = sid_opt.clone();
                                let tn = tc_name.clone();
                                async move {
                                    match sid {
                                        Some(s) => {
                                            crate::commands::mcp::execute_tool_call(
                                                &app, &s, &tn, args, pid_c.as_deref(), &mode_c, &tok_c,
                                            ).await
                                        }
                                        None => Err(format!("工具 {} 的 MCP server 未连接或意外断开，请检查 MCP 服务器状态", tn)),
                                    }
                                }
                            },
                            &ac,
                            &tok,
                        ).await
                    };

                    let (content_text, result_value, is_error) = match tool_result {
                        Ok(tr) => {
                            let text = tr
                                .content
                                .iter()
                                .find(|c| c.kind == "text")
                                .and_then(|c| c.data.get("text"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| {
                                    serde_json::to_string(&tr.content).unwrap_or_default()
                                });
                            let result_value =
                                serde_json::to_value(&tr.content).unwrap_or(json!([]));
                            (text, result_value, tr.is_error)
                        }
                        Err(e) => (format!("[Error] {}", e), json!({ "error": e }), true),
                    };
                    // 对于文件修改工具，捕获 diff 供前端展示
                    let file_changes = file_path_for_diff
                        .and_then(|path| {
                            file_tools::resolve_project_root(&app_c, project_id_c.as_deref())
                                .ok()
                                .and_then(|root| git_tools::capture_file_diff(&root, &path))
                        })
                        .map(|fc| vec![fc]);
                    // 立即发射结果（不等待其他工具）
                    let _ = window.emit(
                        "llm-tool-result",
                        ToolResultPayload {
                            assistant_id: assistant_id_c.clone(),
                            topic_id: topic_id_c.clone(),
                            tool_call_id: tc.id.clone(),
                            name: tc.name.clone(),
                            content: content_text.clone(),
                            result: result_value.clone(),
                            is_error,
                            file_changes,
                            full_content: Some(content_text.clone()),
                        },
                    );

                    // 立即回填 role:tool 消息
                    let mut tool_msg = serde_json::Map::new();
                    tool_msg.insert("role".into(), json!("tool"));
                    tool_msg.insert("content".into(), json!(truncate_tool_result(&content_text)));
                    tool_msg.insert("tool_call_id".into(), json!(tc.id));
                    tool_msg.insert("name".into(), json!(tc.name));
                    messages_for_api.push(serde_json::Value::Object(tool_msg));
                }
            }

            // 阶段 2 — 并发执行所有子 Agent
            if !delegate_futures.is_empty() {
                let mut unordered = FuturesUnordered::new();
                for (i, fut) in delegate_futures {
                    unordered.push(async move {
                        let result = fut.await;
                        (i, result)
                    });
                }

                while let Some((i, result)) = unordered.next().await {
                    let (content_text, result_value, is_error) = match result {
                        Ok(tr) => {
                            let text = tr
                                .content
                                .iter()
                                .find(|c| c.kind == "text")
                                .and_then(|c| c.data.get("text"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string())
                                .unwrap_or_else(|| {
                                    serde_json::to_string(&tr.content).unwrap_or_default()
                                });
                            let result_value =
                                serde_json::to_value(&tr.content).unwrap_or(json!([]));
                            (text, result_value, tr.is_error)
                        }
                        Err(e) => (format!("[Error] {}", e), json!({ "error": e }), true),
                    };
                    // 立即发射该 delegate 结果
                    let tc = &round_result.tool_calls[i];
                    let _ = window.emit(
                        "llm-tool-result",
                        ToolResultPayload {
                            assistant_id: assistant_id_c.clone(),
                            topic_id: topic_id_c.clone(),
                            tool_call_id: tc.id.clone(),
                            name: tc.name.clone(),
                            content: content_text.clone(),
                            result: result_value.clone(),
                            is_error,
                            file_changes: None,
                            full_content: Some(content_text.clone()),
                        },
                    );

                    // 立即回填 role:tool 消息
                    let mut tool_msg = serde_json::Map::new();
                    tool_msg.insert("role".into(), json!("tool"));
                    tool_msg.insert("content".into(), json!(truncate_tool_result(&content_text)));
                    tool_msg.insert("tool_call_id".into(), json!(tc.id));
                    tool_msg.insert("name".into(), json!(tc.name));
                    messages_for_api.push(serde_json::Value::Object(tool_msg));
                }
            }

            // 按原始 index 顺序发射结果 + 回填 role:tool 消息
            for (i, tc) in round_result.tool_calls.iter().enumerate() {
                if token_inner.is_cancelled() {
                    was_cancelled = true;
                    break;
                }
                if let Some(result) = exec_results[i].take() {
                    let _ = window.emit(
                        "llm-tool-result",
                        ToolResultPayload {
                            assistant_id: assistant_id_c.clone(),
                            topic_id: topic_id_c.clone(),
                            tool_call_id: tc.id.clone(),
                            name: tc.name.clone(),
                            content: result.content_text.clone(),
                            result: result.result_value,
                            is_error: result.is_error,
                            file_changes: None,
                            full_content: Some(result.content_text.clone()),
                        },
                    );

                    let mut tool_msg = serde_json::Map::new();
                    tool_msg.insert("role".into(), json!("tool"));
                    tool_msg.insert(
                        "content".into(),
                        json!(truncate_tool_result(&result.content_text)),
                    );
                    tool_msg.insert("tool_call_id".into(), json!(tc.id));
                    tool_msg.insert("name".into(), json!(tc.name));
                    messages_for_api.push(serde_json::Value::Object(tool_msg));
                }
            }

            // 上下文预算检查（token 级、模型感知）：估算当前上下文，超阈值则压缩
            let estimated_tokens: usize =
                estimate_context_tokens(&model, &messages_for_api, context_budget);
            if estimated_tokens > context_budget && messages_for_api.len() > 4 {
                let window_c = window.clone();
                let client_c = client.clone();
                let api_url_c = api_url.clone();
                let api_key_c = api_key.clone();
                let model_c = model.clone();
                let a_id = assistant_id_c.clone();
                let t_id = topic_id_c.clone();
                let tok = token_inner.clone();
                let mut msgs = std::mem::take(&mut messages_for_api);
                match compress_context(
                    &window_c,
                    &client_c,
                    &api_url_c,
                    &api_key_c,
                    &model_c,
                    &mut msgs,
                    &a_id,
                    &t_id,
                    &tok,
                    keep_recent_tokens,
                    "main",
                )
                .await
                {
                    Ok(_) => {
                        compressed_this_run = true;
                        let _ = window.emit(
                            "llm-compression",
                            json!({
                                "round": round,
                                "result": "ok",
                                "estimated_tokens": estimated_tokens,
                                "keep_recent_tokens": keep_recent_tokens,
                            }),
                        );
                    }
                    Err(e) => {
                        let _ = window.emit("llm-compression-failed", json!({"error": e}));
                    }
                }
                messages_for_api = msgs;
            }
        }

        // ===== 未完成任务 → 工作交接文档（handoff）=====
        // 触发条件：强制停止（轮数上限/工作流未调用）、错误、或最后一次响应被截断（length）
        let ended_incomplete = !was_cancelled
            && (incomplete_stop
                || final_error.is_some()
                || last_finish_reason.as_deref() == Some("length"));
        if ended_incomplete {
            match write_handoff_doc(
                &app_c,
                &assistant_id_c,
                &topic_id_c,
                &accumulated_content,
                &messages_for_api,
                &final_error,
            ) {
                Ok(path) => {
                    let reason = if final_error.is_some() {
                        "error"
                    } else if incomplete_stop {
                        "max_rounds"
                    } else {
                        "incomplete"
                    };
                    let _ = window.emit(
                        "llm-handoff",
                        json!({
                            "assistant_id": assistant_id_c,
                            "topic_id": topic_id_c,
                            "path": path,
                            "reason": reason,
                        }),
                    );
                    let note = "\n\n[工作交接] 本次任务未完成，已生成交接文档（含已完成内容与涉及文件，可据此继续）。";
                    accumulated_content.push_str(note);
                    let _ = window.emit(
                        "llm-chunk",
                        StreamPayload {
                            assistant_id: assistant_id_c.clone(),
                            topic_id: topic_id_c.clone(),
                            content: note.into(),
                            done: false,
                            error: None,
                            input_tokens: None,
                            output_tokens: None,
                            context_tokens: None,
                            cached_input_tokens: None,
                            images: None,
                        },
                    );
                    tracing::info!("[handoff] 未完成任务已生成交接文档: {}", path);
                }
                Err(err) => tracing::warn!("[handoff] 生成交接文档失败: {}", err),
            }
        }

        // P2：后台自动事实提取（sleep-time compute）——不阻塞主流程
        if memory_effective && app_config.memory_auto_extract {
            if let Some(pid) = &project_id_c {
                if let Ok(project_root) = file_tools::resolve_project_root(&app_c, Some(pid)) {
                    let transcript = build_extract_transcript(&messages_for_api);
                    let app_x = app_c.clone();
                    let root_x = project_root;
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = crate::services::memory::extract::extract_and_store(
                            &app_x,
                            &root_x,
                            &transcript,
                        )
                        .await
                        {
                            tracing::warn!("[memory] 自动事实提取失败: {e}");
                        }
                    });
                }
            }
        }

        // ===== Epilogue（必达）：无论正常/取消/出错都 emit terminal done =====
        let error_payload = if was_cancelled {
            None
        } else {
            final_error.clone()
        };
        let _ = window.emit(
            "llm-chunk",
            StreamPayload {
                assistant_id: assistant_id_c.clone(),
                topic_id: topic_id_c.clone(),
                content: if let Some(ref e) = final_error {
                    format!("\n[Error: {}]", e)
                } else if !all_images.is_empty() {
                    std::mem::take(&mut accumulated_content)
                } else {
                    "".into()
                },
                done: true,
                error: error_payload,
                input_tokens: if total_input_tokens > 0 {
                    Some(total_input_tokens)
                } else {
                    None
                },
                output_tokens: if total_output_tokens > 0 {
                    Some(total_output_tokens)
                } else {
                    None
                },
                context_tokens: if context_input_tokens > 0 {
                    Some(context_input_tokens)
                } else {
                    None
                },
                cached_input_tokens: if total_cached_tokens > 0 {
                    Some(total_cached_tokens)
                } else {
                    None
                },
                images: if all_images.is_empty() {
                    None
                } else {
                    Some(std::mem::take(&mut all_images))
                },
            },
        );

        // 从全局状态移除自身
        state_inner.remove(&task_key_inner);
    });

    stream_mgr.0.insert(task_key, (handle, token));
    Ok(())
}

/// 为自动事实提取构造对话转写：取最近 12 条非 system 消息，截断到 ~10k 字符。
fn build_extract_transcript(messages: &[serde_json::Value]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let recent: Vec<&serde_json::Value> = messages.iter().rev().take(12).collect();
    for m in recent.iter().rev() {
        let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "system" {
            continue;
        }
        let content = m.get("content").and_then(|v| v.as_str()).unwrap_or("");
        if content.trim().is_empty() {
            continue;
        }
        parts.push(format!("[{role}] {content}"));
    }
    let joined = parts.join("\n");
    joined.chars().take(10_000).collect()
}

/// 上下文自动压缩：将早期消息压缩为摘要，按 token 预算保留最近消息。
/// 用于 Agent 循环中避免超出模型上下文窗口限制。
/// - 前置的 system 消息块（稳定提示词前缀：locale / 知识 / 工作流指令等）永不压缩；
/// - 保留区按 keep_recent_tokens token 预算从尾部累计，并回退到完整轮次边界（不切断 role:tool）；
/// - 摘要输入有界（最多约 30k 字符，最近优先），并固定带上首条 user 消息（原始任务）。
#[allow(
    clippy::too_many_arguments,
    reason = "context compression reuses the active stream request context"
)]
async fn compress_context(
    window: &Window,
    client: &reqwest::Client,
    api_url: &str,
    api_key: &str,
    model: &str,
    messages: &mut Vec<serde_json::Value>,
    assistant_id: &str,
    topic_id: &str,
    token: &CancellationToken,
    keep_recent_tokens: usize,
    source: &str,
) -> Result<(), String> {
    if messages.len() <= 4 {
        return Ok(());
    }

    // 检查取消信号
    if token.is_cancelled() {
        return Err("cancelled".into());
    }

    // 1) 保护前置 system 消息块（稳定提示词前缀），永不压缩
    let mut prefix_end = 0usize;
    while prefix_end < messages.len()
        && messages[prefix_end].get("role").and_then(|v| v.as_str()) == Some("system")
    {
        prefix_end += 1;
    }
    if prefix_end >= messages.len() {
        return Ok(());
    }

    // 2) 按 token 预算从尾部累计保留区
    let mut tail_start = messages.len();
    let mut acc: usize = 0;
    for i in (prefix_end..messages.len()).rev() {
        acc += estimate_message_tokens(model, &messages[i]);
        tail_start = i;
        if acc >= keep_recent_tokens {
            break;
        }
    }
    if tail_start == messages.len() {
        return Ok(()); // 整个可压缩区都在预算内，无需压缩
    }
    // 切割点不得落在 role:tool 上（避免 tool 消息与其 assistant(tool_calls) 分离导致 API 400）
    while tail_start < messages.len()
        && messages[tail_start].get("role").and_then(|v| v.as_str()) == Some("tool")
    {
        tail_start += 1;
    }
    if tail_start <= prefix_end {
        return Ok(()); // 保留区已覆盖全部可压缩区
    }

    let early: Vec<serde_json::Value> = messages.drain(prefix_end..tail_start).collect();

    // 3) 构造有界的摘要提示：首条 user 消息（原始任务）+ 最近的早期消息（最多 ~30k 字符，最近优先）
    let mut blocks: Vec<String> = Vec::new();
    let mut budget = 30_000usize;
    for msg in early.iter().rev() {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("?");
        let text = match msg.get("content") {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => String::new(),
        };
        let text = &text[..text.floor_char_boundary(text.len().min(500))];
        let block = format!("[{}]: {}\n", role, text);
        if block.len() > budget {
            break;
        }
        budget -= block.len();
        blocks.push(block);
    }
    blocks.reverse(); // 恢复时间顺序
                      // 固定带上首条 user 消息（原始任务描述），防止其被预算丢弃
    if let Some(first_user) = early
        .iter()
        .find(|m| m.get("role").and_then(|v| v.as_str()) == Some("user"))
    {
        if let Some(content) = first_user.get("content") {
            let text = match content {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            let text = &text[..text.floor_char_boundary(text.len().min(300))];
            blocks.insert(0, format!("[user]: {}\n", text));
        }
    }

    let mut summary_prompt = String::from(
        "请总结以下 AI 助手对话的历史，保留关键决策、文件路径、代码变更和结论，控制在 2000 字以内：\n\n",
    );
    for b in &blocks {
        summary_prompt.push_str(b);
    }

    let body = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": "你是一个对话摘要助手。请用简洁的中文总结对话要点。"},
            {"role": "user", "content": summary_prompt}
        ],
        "stream": false,
        "max_tokens": 1024
    });
    // 摘要生成；失败（超时/网络/API/空）时回退到本地归档（snapcompact 式：零摘要 API 成本，
    // 细节可经 read_artifact 工具恢复），保证压缩绝不丢失早期上下文。
    let summary = match (async {
        let (res, is_anthropic) = tokio::select! {
            _ = token.cancelled() => return Err("cancelled".to_string()),
            result = tokio::time::timeout(
                std::time::Duration::from_secs(45),
                post_chat_completion(client, api_url, api_key, &body),
            ) => result,
        }
        .map_err(|_| "压缩摘要请求超时（45s）".to_string())?
        .map_err(|e| e.to_string())?;

        let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

        if let Some(err) = chat_error_message(&val) {
            return Err(err);
        }

        let s = chat_text_content(&val, is_anthropic);
        if s.is_empty() {
            Err("摘要为空".to_string())
        } else {
            Ok(s)
        }
    })
    .await
    {
        Ok(s) => s,
        Err(e) if e == "cancelled" => return Err("cancelled".into()),
        Err(e) => {
            let placeholder = archive_early_messages(window, &early, model)?;
            tracing::warn!(
                "[compression] 摘要生成失败（{}），已本地归档 {} 条消息",
                e,
                early.len()
            );
            placeholder
        }
    };

    // 4) 重建消息列表：稳定前缀 → 摘要消息 → 保留尾部
    let tail: Vec<serde_json::Value> = messages.split_off(prefix_end);
    messages.push(json!({
        "role": "system",
        "content": format!("[历史摘要 — 自动压缩]\n{}", summary)
    }));
    messages.extend(tail);

    let _ = window.emit("llm-compression", json!({
        "assistant_id": assistant_id,
        "topic_id": topic_id,
        "source": source,
        "summary": summary,
        "compressed_chars": early.iter().map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0)).sum::<usize>(),
    }));

    Ok(())
}

#[tauri::command]
pub async fn summarize_history(
    api_url: String,
    api_key: String,
    model: String,
    messages_json: String,
) -> Result<String, String> {
    let messages: Vec<serde_json::Value> = serde_json::from_str(&messages_json)
        .map_err(|e| format!("Invalid messages JSON: {}", e))?;
    // 诊断日志
    for (i, m) in messages.iter().enumerate() {
        tracing::info!(
            "[summarize_history] messages[{}] role={} has_tool_call_id={} has_tool_calls={} has_name={}",
            i,
            m.get("role").and_then(|v| v.as_str()).unwrap_or("?"),
            m.get("tool_call_id").is_some(),
            m.get("tool_calls").is_some(),
            m.get("name").is_some()
        );
    }
    // SSRF 防护
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    let client = non_streaming_http_client();

    let mut messages_for_api: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            let content = match &m["content"] {
                serde_json::Value::String(s) => {
                    json!(crate::utils::generated_images::strip_generated_image_tokens(s))
                }
                other => other.clone(),
            };
            json!({ "role": &m["role"], "content": content })
        })
        .collect();

    messages_for_api.push(json!({
        "role": "system",
        "content": "请简要总结以上对话的核心内容和用户需求，作为后续交流的长期记忆（500字以内）。"
    }));

    let body = json!({
        "model": model,
        "messages": messages_for_api,
        "stream": false,
        "max_tokens": 700
    });
    let (res, is_anthropic) = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        post_chat_completion(&client, &api_url, &api_key, &body),
    )
    .await
    .map_err(|_| "摘要请求超时（45s）".to_string())?
    .map_err(|e| e.to_string())?;

    let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

    if let Some(err) = chat_error_message(&val) {
        return Err(err);
    }

    let summary = chat_text_content(&val, is_anthropic);
    let summary = if summary.is_empty() {
        "无法生成总结".to_string()
    } else {
        summary
    };

    Ok(summary)
}

/// 返回按日期降序排列的每日 input/output tokens 与请求次数。
/// 用于前端用量面板展示（如"过去 7 天 / 30 天用量"）。
#[tauri::command]
pub fn get_usage_summary(
    db_state: tauri::State<'_, DbState>,
    days: u32,
) -> Result<Vec<UsageSummary>, String> {
    let conn = db_state.0.lock();
    let mut stmt = conn
        .prepare(
            "SELECT date(timestamp) as day,
                    SUM(input_tokens) as total_input,
                    SUM(output_tokens) as total_output,
                    COALESCE(SUM(cached_input_tokens), 0) as total_cached,
                    COUNT(*) as request_count
             FROM usage_log
             WHERE timestamp >= datetime('now', ?1)
             GROUP BY day
             ORDER BY day DESC",
        )
        .map_err(|e| e.to_string())?;
    let days_param = format!("-{} days", days);
    let rows = stmt
        .query_map([&days_param], |row| {
            Ok(UsageSummary {
                date: row.get(0)?,
                input_tokens: row.get(1)?,
                output_tokens: row.get(2)?,
                cached_input_tokens: row.get(3)?,
                request_count: row.get(4)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut summaries = Vec::new();
    for row in rows {
        summaries.push(row.map_err(|e| e.to_string())?);
    }
    Ok(summaries)
}

/// 查询最近 N 天的 token 用量按模型聚合。
///
/// 返回按总 token 数降序排列的各模型用量，用于前端模型分布面板。
#[tauri::command]
pub fn get_usage_summary_by_model(
    db_state: tauri::State<'_, DbState>,
    days: u32,
    date: Option<String>,
) -> Result<Vec<UsageSummaryByModel>, String> {
    let conn = db_state.0.lock();
    if let Some(ref date_str) = date {
        let mut stmt = conn
            .prepare(
                "SELECT model_id,
                        SUM(input_tokens) as total_input,
                        SUM(output_tokens) as total_output,
                        COALESCE(SUM(cached_input_tokens), 0) as total_cached,
                        COUNT(*) as request_count
                 FROM usage_log
                 WHERE date(timestamp) = ?1
                 GROUP BY model_id
                 ORDER BY (SUM(input_tokens) + SUM(output_tokens)) DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([date_str.as_str()], |row| {
                Ok(UsageSummaryByModel {
                    model_id: row.get(0)?,
                    input_tokens: row.get(1)?,
                    output_tokens: row.get(2)?,
                    cached_input_tokens: row.get(3)?,
                    request_count: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut summaries = Vec::new();
        for row in rows {
            summaries.push(row.map_err(|e| e.to_string())?);
        }
        Ok(summaries)
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT model_id,
                        SUM(input_tokens) as total_input,
                        SUM(output_tokens) as total_output,
                        COALESCE(SUM(cached_input_tokens), 0) as total_cached,
                        COUNT(*) as request_count
                 FROM usage_log
                 WHERE timestamp >= datetime('now', ?1)
                 GROUP BY model_id
                 ORDER BY (SUM(input_tokens) + SUM(output_tokens)) DESC",
            )
            .map_err(|e| e.to_string())?;
        let days_param = format!("-{} days", days);
        let rows = stmt
            .query_map([&days_param], |row| {
                Ok(UsageSummaryByModel {
                    model_id: row.get(0)?,
                    input_tokens: row.get(1)?,
                    output_tokens: row.get(2)?,
                    cached_input_tokens: row.get(3)?,
                    request_count: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?;
        let mut summaries = Vec::new();
        for row in rows {
            summaries.push(row.map_err(|e| e.to_string())?);
        }
        Ok(summaries)
    }
}

#[tauri::command]
pub async fn append_message(
    state: tauri::State<'_, DbState>,
    topic_id: String,
    message: Message,
) -> Result<(), String> {
    let conn = (*state).0.lock();
    let message_id = message
        .id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let files_json = serde_json::to_string(&message.display_files).ok();
    let content_json = serde_json::to_string(&message.content).unwrap_or_default();
    let tool_calls_json = serde_json::to_string(&message.tool_calls).ok();

    conn.execute(
        "INSERT INTO messages
         (id, topic_id, role, content, model_id, display_files, display_text, reasoning,
          tool_call_id, name, tool_calls_json, input_tokens, output_tokens,
          agent_steps_json, interim_content, agent_start_time, parent_message_id, branch_index,
          images_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
        params![
            message_id,
            topic_id,
            message.role,
            content_json,
            message.model_id,
            files_json,
            message.display_text,
            message.reasoning,
            message.tool_call_id,
            message.name,
            tool_calls_json,
            message.input_tokens,
            message.output_tokens,
            serde_json::to_string(&message.agent_steps).ok(),
            message.interim_content,
            message.agent_start_time,
            message.parent_message_id,
            message.branch_index,
            serde_json::to_string(&message.images).ok(),
        ],
    )
    .map_err(|e| e.to_string())?;
    sync_message_attachments(&conn, &message_id, message.display_files.as_ref())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_topic_message(
    state: tauri::State<'_, DbState>,
    topic_id: String,
    message_id: String,
) -> Result<(), String> {
    let conn = (*state).0.lock();
    conn.execute(
        "DELETE FROM messages WHERE id = ?1 AND topic_id = ?2",
        params![message_id, topic_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// 从消息内容中提取纯文本，多模态数组（OpenAI vision 格式）只保留 text 部分。
fn extract_text_content(content: &serde_json::Value) -> String {
    match content {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| {
                if v.get("type")?.as_str()? == "text" {
                    v.get("text")?.as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

/// 清洗模型返回的原始字符串为合规标题。
/// 1. 去除首尾空白与首尾成对引号（半角 / 全角 / 中文书名号 / 反引号）
/// 2. 取第一个非空行（避免多行输出）
/// 3. 递归剥离常见中英文前缀（"标题：" / "Title:" / "好的，" / "以下是" 等）
/// 4. 去除成对 Markdown 标记（**...** / `...`）
fn clean_topic_title(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // 取第一个非空行
    let first_line = trimmed
        .lines()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");

    let mut s = first_line.to_string();

    // 剥离常见前缀（最多尝试 3 轮，防止 "好的，标题是：xxx" 这种嵌套）
    const PREFIXES: &[&str] = &[
        "好的，标题是：",
        "好的，标题是:",
        "好的，标题：",
        "好的，标题:",
        "好的：",
        "好的:",
        "好的，",
        "好的,",
        "标题是：",
        "标题是:",
        "标题：",
        "标题:",
        "Title:",
        "Title：",
        "title:",
        "title：",
        "以下是",
        "以下为",
        "下面给出",
        "给你一个",
        "Here is the title:",
        "Here is the title：",
        "The title is:",
        "The title is：",
    ];
    for _ in 0..3 {
        let mut matched = false;
        for p in PREFIXES {
            if s.starts_with(p) {
                s = s[p.len()..].trim().to_string();
                matched = true;
                break;
            }
        }
        if !matched {
            break;
        }
    }

    // 去除首尾成对引号（中英文 + 反引号 + 书名号）
    s = s
        .trim_matches(|c: char| {
            matches!(
                c,
                '"' | '\''
                    | '`'
                    | '「'
                    | '」'
                    | '『'
                    | '』'
                    | '\u{201C}'
                    | '\u{201D}'
                    | '\u{2018}'
                    | '\u{2019}'
            )
        })
        .to_string();

    // 去除成对 Markdown 标记
    if s.len() > 4 && s.starts_with("**") && s.ends_with("**") {
        s = s[2..s.len() - 2].to_string();
    } else if s.len() > 2 && s.starts_with('`') && s.ends_with('`') {
        s = s[1..s.len() - 1].to_string();
    }

    s.trim().to_string()
}

/// 为话题生成一个简短标题（4-20 个字符）。
/// 由前端在新话题的"第一次对话"后调用一次，生成后前端将 `topic.renamed` 置为 `true`，
/// 后续不再调用以避免重复重命名。
/// 仅做内容生成，不写入数据库 —— 持久化由前端在更新 Store 后通过 `save_assistant` 完成。
///
/// # 参数
/// - `api_url` / `api_key` / `model`：调用方所用的 LLM 凭据（与流式对话保持一致）
/// - `messages`：用于生成标题的对话内容（建议取前 2~4 条）
///
/// # 返回
/// 成功时返回清洗后的标题字符串（已去除引号、空白、换行与常见前缀，长度限制在 1-20 字符内）。
///
/// # 失败模式
/// 若 LLM 长时间返回空内容（finish_reason=stop 且 content 为空），错误信息会附带
/// 模型名与原始长度，便于排查。前端应在 catch 中走启发式后备方案。
#[tauri::command]
pub async fn generate_topic_title(
    api_url: String,
    api_key: String,
    model: String,
    messages_json: String,
) -> Result<String, String> {
    let messages: Vec<serde_json::Value> = serde_json::from_str(&messages_json)
        .map_err(|e| format!("Invalid messages JSON: {}", e))?;
    // 诊断日志
    for (i, m) in messages.iter().enumerate() {
        tracing::info!(
            "[generate_topic_title] messages[{}] role={} has_tool_call_id={} has_tool_calls={} has_name={}",
            i,
            m.get("role").and_then(|v| v.as_str()).unwrap_or("?"),
            m.get("tool_call_id").is_some(),
            m.get("tool_calls").is_some(),
            m.get("name").is_some()
        );
    }
    // SSRF 防护
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    if messages.is_empty() {
        return Err("生成标题需要至少一条消息".to_string());
    }

    let client = non_streaming_http_client();

    let mut messages_for_api: Vec<serde_json::Value> = vec![json!({
        "role": "system",
        "content": "你是一个话题标题生成助手，擅长用最少的字数精准概括对话核心内容。"
    })];

    for m in &messages {
        let text = match &m["content"] {
            serde_json::Value::String(s) => {
                crate::utils::generated_images::strip_generated_image_tokens(s)
            }
            other => extract_text_content(other),
        };
        if text.trim().is_empty() {
            continue;
        }
        messages_for_api.push(json!({ "role": &m["role"], "content": text }));
    }

    messages_for_api.push(json!({
        "role": "user",
        "content": "请根据以上对话生成一个 4-20 字的话题标题。\n\
                     严格要求：\n\
                     1. 精准概括核心主题或关键问题\n\
                     2. 不要加引号、冒号、序号、'好的'、'以下是'等多余文字\n\
                     3. 不要使用任何 Markdown 标记\n\
                     4. 你的回复必须且只能包含标题本身"
    }));

    let body = json!({
        "model": model,
        "messages": messages_for_api,
        "stream": false,
        "max_tokens": 200,
        "temperature": 0.0
    });

    let (res, is_anthropic) = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        post_chat_completion(&client, &api_url, &api_key, &body),
    )
    .await
    .map_err(|_| "标题生成请求超时（45s）".to_string())?
    .map_err(|e| e.to_string())?;

    let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

    if let Some(err) = chat_error_message(&val) {
        return Err(err);
    }

    let raw = chat_text_content(&val, is_anthropic);

    let cleaned = clean_topic_title(&raw);

    if cleaned.is_empty() {
        let finish = chat_finish_reason(&val, is_anthropic);
        return Err(format!(
            "模型 {} 返回的标题为空 (finish_reason={}, raw_len={})",
            model,
            finish,
            raw.len()
        ));
    }

    let truncated: String = if cleaned.chars().count() > 20 {
        cleaned.chars().take(20).collect()
    } else {
        cleaned
    };

    Ok(truncated)
}

/// 估算文本的 token 数（供前端输入框实时预览）。
/// 包装调用 `utils::token_counter::count_tokens_cmd` 纯函数。
#[tauri::command]
pub fn count_tokens_cmd(model: String, text: String) -> Result<usize, String> {
    crate::utils::token_counter::count_tokens_cmd(model, text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::GeneratedImage;

    fn conn() -> rusqlite::Connection {
        rusqlite::Connection::open_in_memory().unwrap()
    }

    #[test]
    fn message_for_api_expands_images_and_strips_token() {
        let dir = std::env::temp_dir().join(format!("aio-msg-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let img_path = dir.join("g.png");
        std::fs::write(&img_path, b"\x89PNG").unwrap();
        let abs = img_path
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let token = crate::utils::generated_images::image_token_for(&abs);
        let content_html = format!("text before ![img]({})", token);
        let msg = Message {
            id: None,
            role: "assistant".into(),
            content: serde_json::json!(content_html),
            model_id: None,
            display_files: None,
            display_text: None,
            tool_call_id: None,
            name: None,
            tool_calls: None,
            reasoning: None,
            input_tokens: None,
            output_tokens: None,
            agent_steps: None,
            interim_content: None,
            agent_start_time: None,
            parent_message_id: None,
            branch_index: 0,
            images: Some(vec![GeneratedImage {
                name: "g.png".into(),
                mime_type: "image/png".into(),
                size: 4,
                storage_path: abs.clone(),
            }]),
            full_tool_result: None,
        };
        let out = message_for_api(&conn(), &msg).unwrap();
        let obj = out.as_object().unwrap();
        let parts = obj.get("content").unwrap().as_array().unwrap();
        // 文本部分已剥离 aio-image 标记（防 token 泄漏）
        let text = parts[0]["text"].as_str().unwrap();
        assert!(!text.contains("aio-image://"));
        assert!(text.contains("text before"));
        // 图像部分重新展开为 image_url data URI（历史回放）
        assert_eq!(parts[1]["type"], "image_url");
        let url = parts[1]["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
