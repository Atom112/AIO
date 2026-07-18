use crate::core::permission::{self, PermissionAction};
use crate::core::state::DbState;
use crate::core::state::PendingApprovals;
use crate::commands::attachment::sync_message_attachments;
use crate::core::state::McpServerState;
use crate::core::subagent;
use crate::plugins::mcp::McpServerManager;
use crate::utils::file_tools;
use crate::utils::git_tools;
use crate::utils::lsp_tools;
use crate::utils::shell_tools;
use crate::utils::web_tools;
use base64::{engine::general_purpose, Engine as _};
use rusqlite::params;
use crate::core::models::*;
use crate::core::state::StreamManager;
use futures_util::StreamExt; // 用于处理流式数据
use futures_util::stream::FuturesUnordered;
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::collections::HashMap;
use tauri::{AppHandle, Emitter, Manager, Window}; // Emitter 用于从后端向前端推送事件
use tokio_util::sync::CancellationToken;

/// 流式 HTTP 客户端（LLM 推理专用）。
///
/// - connect_timeout(5s)：快速检测不可达服务器
/// - timeout(600s)：10 分钟硬上限，防止极端慢推理导致无限流
/// - tcp_keepalive(30s)：检测 TCP 层网络分区
/// - http2_keep_alive_interval(30s)：检测 HTTP/2 连接静默断开
///
/// Chunk 级 inactivity 超时（120s）在 `stream_one_round()` 循环中独立处理。
fn streaming_http_client() -> reqwest::Client {
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
fn non_streaming_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// 单轮流式调用后的累积结果。
struct RoundResult {
    /// 本轮 assistant 文本内容
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
                parts.extend(image_data_urls.into_iter().map(|url| {
                    json!({ "type": "image_url", "image_url": { "url": url } })
                }));
                serde_json::Value::Array(parts)
            };
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
fn insert_usage_log(
    app: &AppHandle,
    assistant_id: &str,
    topic_id: &str,
    model_id: &str,
    round: u32,
    input_tokens: u32,
    output_tokens: u32,
) {
    if input_tokens == 0 && output_tokens == 0 {
        return;
    }
    let db = app.state::<DbState>();
    let conn = match db.0.lock() {
        Ok(c) => c,
        Err(_) => return,
    };
    let id = uuid::Uuid::new_v4().to_string();
    let _ = conn.execute(
        "INSERT INTO usage_log (id, assistant_id, topic_id, model_id, round, input_tokens, output_tokens) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![id, assistant_id, topic_id, model_id, round, input_tokens, output_tokens],
    );
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
async fn stream_one_round(
    window: &Window,
    token: &CancellationToken,
    client: &reqwest::Client,
    mut api_url: String,
    api_key: &str,
    model: &str,
    messages: &[serde_json::Value],
    tools: Option<&[ToolSpec]>,
    assistant_id: &str,
    topic_id: &str,
    suppress_events: bool,
) -> Result<RoundResult, String> {
    api_url = api_url.trim_end_matches('/').to_string();
    let final_url = if !api_url.ends_with("/chat/completions") {
        format!("{}/chat/completions", api_url)
    } else {
        api_url
    };

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
    let body = serde_json::Value::Object(body_map);

    let response = client
        .post(&final_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        let truncated = if body_text.len() > 512 { &body_text[..512] } else { &body_text };
        return Err(format!("LLM API {}: {}", status, truncated));
    }

    let mut stream = response.bytes_stream();
    let mut line_buffer = String::new();
    // tool_call 累积：index → (id, name, arguments)，用 BTreeMap 保证按 index 升序 flush
    let mut tc_accum: BTreeMap<usize, (String, String, String)> = BTreeMap::new();

    let mut content_buf = String::new();
    let mut reasoning_buf = String::new();
    let mut saw_done = false;
    // 从 SSE 流末尾提取 token 用量（服务端返回）
    let mut input_tokens: u32 = 0;
    let mut output_tokens: u32 = 0;

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

            if line.starts_with("data: ") {
                let json_str = &line[6..];
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
                    if let Some(content) = val["choices"][0]["delta"]["content"].as_str() {
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
                                },
                            );
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
                                    },
                                );
                            }
                        }
                    }
                    // tool_calls 累积
                    if let Some(tcs) = val["choices"][0]["delta"]["tool_calls"].as_array() {
                        for tc in tcs {
                            let index = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
                            let entry = tc_accum.entry(index).or_insert_with(|| {
                                (String::new(), String::new(), String::new())
                            });
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
                    if finish == "tool_calls" {
                        if !suppress_events {
                            for (_idx, (id, name, args)) in tc_accum.iter() {
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
                    }
                    // 提取服务端返回的 token 用量（OpenAI 兼容 API 在最后一个 chunk 中附带 usage）
                    if let Some(usage) = val.get("usage") {
                        if let Some(pt) = usage.get("prompt_tokens").and_then(|v| v.as_u64()) {
                            input_tokens = pt as u32;
                        }
                        if let Some(ct) = usage.get("completion_tokens").and_then(|v| v.as_u64()) {
                            output_tokens = ct as u32;
                        }
                    }
                }
            }
        }
        if saw_done {
            break;
        }
    }

    // 按 index 升序构造工具调用列表（若 finish_reason="tool_calls" 已 emit 过通知，这里不再重复 emit）
    let mut tool_calls = Vec::new();
    let need_emit = !saw_done; // 若未到 [DONE]，则此前可能未 emit tool-call 通知
    for (_idx, (id, name, args)) in tc_accum.iter() {
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

    Ok(RoundResult {
        content: content_buf,
        reasoning: reasoning_buf,
        tool_calls,
        input_tokens,
        output_tokens,
    })
}

/// 为助手构建本轮可用工具（复用 list_mcp_tools_for_assistant 核心逻辑）。
///
/// 返回 `(tools, tool_server_map)`。`plan` 模式或空 server 列表返回空（无工具注入）。
async fn build_tools_for_assistant(
    app: &AppHandle,
    mgr: &McpServerManager,
    state: &McpServerState,
    mcp_server_ids: &[String],
    project_id: Option<&str>,
) -> (Vec<ToolSpec>, std::collections::HashMap<String, String>) {
    if mcp_server_ids.is_empty() {
        return (Vec::new(), std::collections::HashMap::new());
    }
    match crate::commands::mcp::list_mcp_tools_for_assistant_inner(app, mgr, state, mcp_server_ids.to_vec(), project_id.map(|s| s.to_string())).await {
        Ok(at) => (at.tools, at.tool_server_map),
        Err(_) => (Vec::new(), std::collections::HashMap::new()),
    }
}

/// 核心函数：调用 LLM 并分块回传结果（流式输出）。
///
/// 注意：此命令保留用于命令稳定性，但前端新流程改用 [`run_agent_turn`]
/// （后端单任务自驱循环）。本命令现在仅做单轮流式 + 终止 done，
/// 不再做工具执行/递归——工具调用的通知事件仍会 emit（供调试/兼容）。
#[tauri::command]
pub async fn call_llm_stream(
    window: Window,                         // Tauri 窗口句柄，用于发送事件
    state: tauri::State<'_, StreamManager>, // 全局状态，用于管理正在进行的流任务
    db_state: tauri::State<'_, DbState>,
    api_url: String,                        // API 地址
    api_key: String,                        // API 密钥
    model: String,                          // 模型名称（如 gpt-3.5-turbo）
    assistant_id: String,                   // 助手 ID（用于前端匹配消息）
    topic_id: String,                       // 话题/会话 ID
    messages: Vec<Message>,                 // 历史上下文消息列表
    tools: Option<Vec<ToolSpec>>,           // 工具定义（MCP 工具，None 或空数组则不发送）
) -> Result<(), String> {
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
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;
        messages
            .iter()
            .map(|message| message_for_api(&conn, message))
            .collect::<Result<Vec<_>, _>>()?
    };
    let tools_slice = tools.map(|t| t); // 用于 as_slice()

    // 防御性校验：检查 tool_calls 与 tool 响应是否匹配
    verify_tool_messages(&messages_for_api);

    let token = CancellationToken::new();
    let token_inner = token.clone();
    let app_handle = window.app_handle().clone();
    let model_c = model.clone();

    // 4. 创建异步任务执行请求
    let handle = tokio::spawn(async move {
        let client = streaming_http_client();
        let tools_ref: Option<&[ToolSpec]> = tools_slice.as_ref().map(|v| v.as_slice());
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
                );
                let _ = window.emit(
                    "llm-chunk",
                    StreamPayload {
                        assistant_id: assistant_id_c.clone(),
                        topic_id: topic_id_c.clone(),
                        content: "".into(),
                        done: true,
                        error: None,
                        input_tokens: Some(round.input_tokens),
                        output_tokens: Some(round.output_tokens),
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
                            reason = format!("命令: {cmd}\n\n工具 '{}' 需要您的确认才能执行", tool_name);
                        }
                    }
                }
                let approval_fut = crate::commands::mcp::request_tool_approval(
                    app,
                    pending.inner(),
                    "__aio-filesystem__",
                    tool_name,
                    arguments,
                    &reason,
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
        .unwrap_or_else(|_| {
            shell_tools::tool_err("命令执行线程 panic")
        }))
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
    } else if tool_name.starts_with("git_") {
        let tool_name_c = tool_name.to_string();
        let arguments_c = arguments.clone();
        let root_c = project_root.clone();
        Ok(tokio::task::spawn_blocking(move || {
            git_tools::execute_git_tool(&tool_name_c, &arguments_c, &root_c)
        })
        .await
        .unwrap_or_else(|_| {
            git_tools::tool_err("Git 工具执行线程 panic")
        }))
    } else {
        let tool_name_c = tool_name.to_string();
        let arguments_c = arguments.clone();
        let root_c = project_root.clone();
        Ok(tokio::task::spawn_blocking(move || {
            file_tools::execute_file_tool(&tool_name_c, &arguments_c, &root_c)
        })
        .await
        .unwrap_or_else(|_| {
            file_tools::tool_err("文件工具执行线程 panic")
        }))
    }
}

/// 截断文本用于显示（保留前 max_len 字符并追加 "…"）
fn truncate_for_display(s: &str, max_len: usize) -> String {
    if s.chars().count() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s.chars().take(max_len).collect::<String>())
    }
}

/// 构建子 Agent 的消息列表（系统提示词 + 任务描述 + 上下文提示）
fn build_subagent_messages(
    profile: &subagent::SubagentProfile,
    task_desc: &str,
    context_files: &[String],
    project_root: &str,
) -> Vec<serde_json::Value> {
    let system_prompt = format!(
        concat!(
            "你是 AIO 的子智能体，类型为「{}」。\n",
            "工作目录: {}\n",
            "请高效完成子任务并在最后总结工作成果。\n\n",
            "{}\n\n",
            "重要：请在回复末尾给出简洁的工作总结（含修改的文件路径和关键发现）。回复请保持精炼，重点突出核心成果，避免冗长叙述。"
        ),
        profile.name,
        project_root,
        profile.system_prompt_extension,
    );

    let mut msgs: Vec<serde_json::Value> = vec![
        json!({"role": "system", "content": system_prompt}),
        json!({"role": "user", "content": format!("子任务:\n{}\n\n请开始执行。完成后在最后给出工作总结。", task_desc)}),
    ];

    if !context_files.is_empty() {
        let files_hint = context_files
            .iter()
            .map(|f| format!("- {}", f))
            .collect::<Vec<_>>()
            .join("\n");
        msgs.push(json!({
            "role": "user",
            "content": format!("提示：以下文件可能与任务相关，请根据需要读取：\n{}", files_hint)
        }));
    }

    msgs
}

/// 构建子 Agent 的受限工具列表（根据 profile 过滤）
fn build_subagent_tools(profile: &subagent::SubagentProfile, project_root: &str) -> (Vec<ToolSpec>, HashMap<String, String>) {
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
    // LSP 工具
    if profile.is_tool_allowed("read_lints") {
        let spec = lsp_tools::tool_spec();
        tool_map.insert(spec.function.name.clone(), "__builtin__".into());
        tools.push(spec);
    }

    (tools, tool_map)
}

/// Helper: check if a directory is a git repo
fn is_git_repo(dir: &str) -> bool {
    std::path::Path::new(dir).join(".git").exists()
}

/// 截断过长的工具返回内容，防止 LLM 上下文膨胀。完整内容保留在 agentSteps 中供用户查看。
/// MAX_LEN = 10000 字符，覆盖大多数工具返回（代码片段、文件列表、搜索结果），约占 ~2500 tokens。
fn truncate_tool_result(s: &str) -> String {
    const MAX_LEN: usize = 10000;
    if s.len() <= MAX_LEN {
        s.to_string()
    } else {
        format!("{}\n\n... [已截断: 共{}字符]", &s[..MAX_LEN], s.len())
    }
}

/// 处理 delegate_task 工具调用（从主 Agent 循环中调用）。
///
/// 解析参数、验证权限、获取 profile，然后委托给 execute_subagent 执行。
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
    token: &CancellationToken,
    profile_model_overrides: &[ProfileModelOverride],
    custom_profiles: &[crate::core::models::CustomSubagentProfile],
) -> Result<ToolResult, String> {
    // 解析参数
    let profile_id = arguments["profile"].as_str().unwrap_or("general");
    let task_desc = arguments["task"].as_str().unwrap_or("").to_string();
    if task_desc.is_empty() {
        return Err("delegate_task 缺少必填参数 'task'".into());
    }
    let profile = subagent::find_profile(profile_id, custom_profiles)
        .ok_or_else(|| format!("未知的子智能体类型: '{}'，可用: explorer, coder, general, architect, debugger, reviewer, writer, tester, requirements, 或自定义角色 ID", profile_id))?;

    // 解析 per-profile 模型覆盖
    let override_info = profile_model_overrides.iter().find(|o| o.profile_id == profile.id);
    let (resolved_api_url, resolved_api_key, resolved_model) = if let Some(ov) = override_info {
        (ov.api_url.as_str(), ov.api_key.as_str(), ov.model_id.as_str())
    } else {
        (api_url, api_key, model)
    };

    let context_files: Vec<String> = arguments["context_files"]
        .as_array()
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
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
        &task_desc,
        &context_files,
        &project_root,
        project_id,
        agent_mode,
        token,
    )
    .await
}

/// 执行一个工作流（按顺序依次运行每个步骤的子智能体）。
///
/// 每个步骤独立调用 `handle_delegate_task`，前一步的输出作为上下文追加到下一步的任务描述中。
/// 通过 Tauri 事件向前端推送工作流进度（workflow-start / workflow-step-start / workflow-step-complete / workflow-complete）。
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
    custom_profiles: &[CustomSubagentProfile],
    profile_model_overrides: &[ProfileModelOverride],
    token: &CancellationToken,
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
            let _ = window.emit("workflow-complete", serde_json::json!({ "workflowId": workflow.workflow_id }));
            return Err("工作流被用户取消".into());
        }

        // 发射 workflow-step-start 事件
        let _ = window.emit("workflow-step-start", serde_json::json!({ "stepId": step.step_id }));

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
            token,
            profile_model_overrides,
            custom_profiles,
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

                // 追加到上下文
                context.push_str(&format!("\n## 步骤 {} ({}): {}\n{}",
                    idx + 1, step.name, step.profile_id, step_result_text));
                all_results.push(format!("步骤 {} ({}): {}",
                    idx + 1, step.name, step_result_text));

                // 发射 workflow-step-complete 事件
                let _ = window.emit("workflow-step-complete", serde_json::json!({
                    "stepId": step.step_id,
                    "status": "completed",
                    "duration": duration_ms,
                }));
            }
            Err(e) => {
                context.push_str(&format!("\n## 步骤 {} ({}): [失败] {}", idx + 1, step.name, e));
                all_results.push(format!("步骤 {} ({}): [失败] {}", idx + 1, step.name, e));

                // 发射 workflow-step-complete 事件（失败）
                let _ = window.emit("workflow-step-complete", serde_json::json!({
                    "stepId": step.step_id,
                    "status": "failed",
                    "duration": duration_ms,
                }));

                // 步骤失败不中断整个工作流，继续执行后续步骤
            }
        }
    }

    // 发射 workflow-complete 事件
    let _ = window.emit("workflow-complete", serde_json::json!({ "workflowId": workflow.workflow_id }));

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
    project_root: &str,
    project_id: Option<&str>,
    agent_mode: &AgentMode,
    token: &CancellationToken,
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

    // 构建子 Agent 的独立消息上下文
    let mut sub_msgs = build_subagent_messages(profile, task_desc, context_files, project_root);
    let (sub_tools, _sub_tool_map) = build_subagent_tools(profile, project_root);
    let tools_slice: Option<&[ToolSpec]> = if sub_tools.is_empty() { None } else { Some(&sub_tools) };

    let mut round: usize = 0;
    let mut final_text = String::new();

    loop {
        round += 1;
        if token.is_cancelled() {
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
        )
        .await;

        match result {
            Ok(rr) => {
                // 追加 assistant 消息到子 Agent 上下文
                let mut asst_obj = serde_json::Map::new();
                asst_obj.insert("role".into(), json!("assistant"));
                asst_obj.insert("content".into(), json!(rr.content));
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
                        step_type: if rr.tool_calls.is_empty() { "content".to_string() } else { "tool_call".to_string() },
                        summary: step_content,
                    },
                );

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
                        let err_text = format!("[已阻止] 子 Agent ({}) 不允许使用工具: {}", profile.id, tc.name);
                        sub_msgs.push(json!({
                            "role": "tool",
                            "content": err_text,
                            "tool_call_id": tc.id,
                            "name": tc.name,
                        }));
                        continue;
                    }

                    let tool_result = execute_builtin_tool(
                        app,
                        &tc.name,
                        &args_val,
                        project_id,
                        &AgentMode::Auto, // 子 Agent 内部固定 Auto，避免权限审批阻塞
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
                                .unwrap_or_else(|| serde_json::to_string(&tr.content).unwrap_or_default());
                            let result_value = serde_json::to_value(&tr.content).unwrap_or(json!([]));
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

                    // 回填 tool 消息
                    sub_msgs.push(json!({
                        "role": "tool",
                        "content": content_text,
                        "tool_call_id": tc.id,
                        "name": tc.name,
                    }));
                }
            }
            Err(e) => {
                if e == "cancelled" {
                    break;
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
        "[子 Agent 未产出最终文本]".to_string()
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
/// - `mcp_server_ids`：助手启用的 MCP server id 列表（opt-in，空 = 无工具）
/// - `agent_mode`：Agent 执行模式，影响权限规则与工具注入（plan 整轮无工具）
/// - `project_id`：项目 id（用于解析项目级权限规则）
#[tauri::command]
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
    mcp_server_ids: Vec<String>,
    agent_mode: AgentMode,
    project_id: Option<String>,
    web_search_enabled: bool,
    profile_model_overrides: Vec<ProfileModelOverride>,
    custom_subagent_profiles: Vec<crate::core::models::CustomSubagentProfile>,
) -> Result<(), String> {
    // SSRF 防护
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    let task_key = format!("{}-{}", assistant_id, topic_id);

    // 取消同 topic 的旧任务（cancel 而非 abort）
    if let Some((_, (_, old_token))) = stream_mgr.0.remove(&task_key) {
        old_token.cancel();
    }

    // 预先把 messages 转为 API 格式（含附件 image 展开等），在持锁期间完成同步 I/O
    let mut messages_for_api: Vec<serde_json::Value> = {
        let conn = db_state.0.lock().map_err(|e| e.to_string())?;
        messages
            .iter()
            .map(|m| message_for_api(&conn, m))
            .collect::<Result<Vec<_>, _>>()?
    };
    verify_tool_messages(&messages_for_api);

    let is_agent_mode = agent_mode != AgentMode::Off;

    let token = CancellationToken::new();
    let token_inner = token.clone();
    let state_inner = stream_mgr.0.clone();
    let task_key_inner = task_key.clone();
    let assistant_id_c = assistant_id.clone();
    let topic_id_c = topic_id.clone();
    let app_c = app.clone();
    let mcp_server_ids_c = mcp_server_ids.clone();
    let project_id_c = project_id.clone();
    let profile_overrides_c = profile_model_overrides.clone();
    let custom_profiles_c = custom_subagent_profiles.clone();

    let handle = tokio::spawn(async move {
        let client = streaming_http_client();

        // 构建工具：仅 Agent 模式（非 Off）且非 Plan 时才注入工具。
        // Off（纯对话）模式绝不向模型暴露工具，避免模型擅自调用；Plan 模式整轮不注入工具。
        // 内置文件工具始终注入（in-process 直接调用，无需 MCP 子进程连接）。
        // 在 spawn 内通过 AppHandle 解析全局状态，避免 tauri::State 借用逃逸。
        let tools_enabled = is_agent_mode && agent_mode != AgentMode::Plan;
        // 联网搜索开关：即使对话模式下也注入 web_fetch + web_search
        let web_only = web_search_enabled && !tools_enabled;
        let (tools, tool_server_map) = if tools_enabled {
            let (mut mcp_tools, mut mcp_map) = if !mcp_server_ids_c.is_empty() {
                let mgr = app_c.state::<McpServerManager>();
                let mcp_state = app_c.state::<McpServerState>();
                build_tools_for_assistant(
                    &app_c,
                    mgr.inner(),
                    mcp_state.inner(),
                    &mcp_server_ids_c,
                    project_id_c.as_deref(),
                )
                .await
            } else {
                (Vec::new(), std::collections::HashMap::new())
            };
            // 始终注入内置工具（in-process，跳过 MCP 子进程）
            let file_specs = file_tools::get_file_tool_specs();
            for spec in &file_specs {
                mcp_map.insert(spec.function.name.clone(), "__builtin__".into());
            }
            mcp_tools.extend(file_specs);
            // 注入命令执行工具
            let cmd_spec = shell_tools::get_command_tool_spec();
            mcp_map.insert(cmd_spec.function.name.clone(), "__builtin__".into());
            mcp_tools.push(cmd_spec);
            // 注入 Web 工具
            let web_specs = web_tools::get_web_tool_specs();
            for spec in &web_specs {
                mcp_map.insert(spec.function.name.clone(), "__builtin__".into());
            }
            mcp_tools.extend(web_specs);
            // 注入 Git 工具（仅当项目是 git 仓库时有效工具）
            let git_specs = git_tools::get_git_tool_specs();
            for spec in &git_specs {
                mcp_map.insert(spec.function.name.clone(), "__builtin__".into());
            }
            mcp_tools.extend(git_specs);
            // 注入 LSP 诊断工具
            let lsp_spec = lsp_tools::tool_spec();
            mcp_map.insert(lsp_spec.function.name.clone(), "__builtin__".into());
            mcp_tools.push(lsp_spec);
            // 注入子智能体委托工具（仅 Agent 模式下可用）
            let delegate_spec = subagent::delegate_task_tool_spec();
            mcp_map.insert(delegate_spec.function.name.clone(), "__builtin__".into());
            mcp_tools.push(delegate_spec);
            // 注入工作流创建工具（仅 Agent 模式下可用）
            let workflow_spec = subagent::create_workflow_tool_spec();
            mcp_map.insert(workflow_spec.function.name.clone(), "__builtin__".into());
            mcp_tools.push(workflow_spec);
            (mcp_tools, mcp_map)
        } else if web_only {
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
        let tools_slice: Option<&[ToolSpec]> = if tools.is_empty() { None } else { Some(&tools) };

        let mut round: u32 = 0;
        let mut final_error: Option<String> = None;
        let mut was_cancelled = false;
        // Workflow 模式：标记是否已调用 create_workflow
        let mut workflow_called = false;
        // 跨轮累计 token 用量
        let mut total_input_tokens: u32 = 0;
        let mut total_output_tokens: u32 = 0;

        // Workflow 模式：在 LLM 循环前注入强制系统消息
        if agent_mode == AgentMode::Workflow {
            messages_for_api.push(serde_json::json!({
                "role": "system",
                "content": concat!(
                    "你正处于「工作流模式」。\n",
                    "你的首要任务：分析用户请求 → 调用 `create_workflow` 工具来创建和执行工作流。\n",
                    "不允许直接修改文件、执行命令或调用其他工具。\n",
                    "必须使用 create_workflow 来组织多步骤任务。\n\n",
                    "典型序列示例：\n",
                    "- requirements → coder → reviewer（分析 + 实现 + 审查）\n",
                    "- explorer → coder（探索 + 实现）\n",
                    "- debugger → coder（诊断 + 修复）\n",
                    "- requirements → architect → coder → tester（完整开发流程）\n\n",
                    "如果用户请求很简单，可以只用一个步骤的工作流。\n",
                    "请立即调用 create_workflow 来开始工作。"
                )
            }));
        }

        'outer: loop {
            round += 1;
            if token_inner.is_cancelled() {
                was_cancelled = true;
                break;
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
            )
            .await;

            let round_result = match round_result {
                Ok(r) => r,
                Err(e) => {
                    if e == "cancelled" {
                        was_cancelled = true;
                    } else {
                        final_error = Some(e);
                    }
                    break 'outer;
                }
            };

            // 累计 token 用量
            total_input_tokens += round_result.input_tokens;
            total_output_tokens += round_result.output_tokens;

            // 持久化本轮 token 用量到 usage_log
            insert_usage_log(
                &app_c,
                &assistant_id_c,
                &topic_id_c,
                &model,
                round,
                round_result.input_tokens,
                round_result.output_tokens,
            );

            // 把本轮 assistant 消息（含 tool_calls）append 到上下文
            let mut asst_obj = serde_json::Map::new();
            asst_obj.insert("role".into(), json!("assistant"));
            asst_obj.insert(
                "content".into(),
                if round_result.content.is_empty() {
                    json!(null)
                } else {
                    json!(round_result.content)
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

            // 无工具调用 → 任务完成，整轮结束
            if round_result.tool_calls.is_empty() {
                if agent_mode == AgentMode::Workflow && !workflow_called {
                    // Workflow 模式下不允许退出——注入强制消息后继续
                    messages_for_api.push(serde_json::json!({
                        "role": "system",
                        "content": "【工作流模式强制指令】你尚未调用 create_workflow 工具。请立即分析用户请求并调用 create_workflow 来创建和执行工作流。"
                    }));
                    continue;
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
                            &token_clone,
                            &profile_overrides_clone,
                            &custom_profiles_clone,
                        )
                        .await
                    });
                    delegate_futures.push((i, fut));
                } else if server_id.as_deref() == Some("__builtin__") && tc.name == "create_workflow" {
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
                                let profile = step["profile"].as_str().unwrap_or("general").to_string();
                                let name = step["name"].as_str().unwrap_or("").to_string();
                                let task = step["task"].as_str().unwrap_or("").to_string();
                                if task.is_empty() {
                                    parse_error = Some(format!("create_workflow: 步骤 {} 缺少 'task' 字段", idx + 1));
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
                                &custom_profiles_c,
                                &profile_overrides_c,
                                &token_inner,
                            ).await
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
                                .unwrap_or_else(|| serde_json::to_string(&tr.content).unwrap_or_default());
                            let result_value = serde_json::to_value(&tr.content).unwrap_or(json!([]));
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
                    let tool_result = if server_id.as_deref() == Some("__builtin__") {
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
                        match server_id {
                            Some(sid) => {
                                crate::commands::mcp::execute_tool_call(
                                    &app_c,
                                    &sid,
                                    &tc.name,
                                    args_val,
                                    project_id_c.as_deref(),
                                    &agent_mode,
                                    &token_inner,
                                )
                                .await
                            }
                            None => Err(format!("未找到工具 {} 对应的 MCP server", tc.name)),
                        }
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
                                .unwrap_or_else(|| serde_json::to_string(&tr.content).unwrap_or_default());
                            let result_value = serde_json::to_value(&tr.content).unwrap_or(json!([]));
                            (text, result_value, tr.is_error)
                        }
                        Err(e) => (format!("[Error] {}", e), json!({ "error": e }), true),
                    };
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
                                .unwrap_or_else(|| serde_json::to_string(&tr.content).unwrap_or_default());
                            let result_value = serde_json::to_value(&tr.content).unwrap_or(json!([]));
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
                        },
                    );

                    let mut tool_msg = serde_json::Map::new();
                    tool_msg.insert("role".into(), json!("tool"));
                    tool_msg.insert("content".into(), json!(truncate_tool_result(&result.content_text)));
                    tool_msg.insert("tool_call_id".into(), json!(tc.id));
                    tool_msg.insert("name".into(), json!(tc.name));
                    messages_for_api.push(serde_json::Value::Object(tool_msg));
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
                } else {
                    "".into()
                },
                done: true,
                error: error_payload,
                input_tokens: if total_input_tokens > 0 { Some(total_input_tokens) } else { None },
                output_tokens: if total_output_tokens > 0 { Some(total_output_tokens) } else { None },
            },
        );

        // 从全局状态移除自身
        state_inner.remove(&task_key_inner);
    });

    stream_mgr.0.insert(task_key, (handle, token));
    Ok(())
}

#[tauri::command]
pub async fn summarize_history(
    api_url: String,
    api_key: String,
    model: String,
    messages: Vec<Message>,
) -> Result<String, String> {
    // SSRF 防护
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    let client = non_streaming_http_client();

    let mut messages_for_api: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| json!({ "role": m.role, "content": m.content }))
        .collect();

    messages_for_api.push(json!({
        "role": "system",
        "content": "请简要总结以上对话的核心内容和用户需求，作为后续交流的长期记忆（500字以内）。"
    }));

    let body = json!({
        "model": model,
        "messages": messages_for_api,
        "stream": false
    });

    // --- 修复后的 URL 拼接逻辑 ---
    let base_url = api_url
        .trim_end_matches('/')
        .replace("/chat/completions", "");
    let endpoint = format!("{}/chat/completions", base_url);

    let res = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        client
            .post(endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send(),
    )
    .await
    .map_err(|_| "摘要请求超时（45s）".to_string())?
    .map_err(|e| e.to_string())?;

    let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

    // 增加一个简单的错误检查
    if let Some(err) = val.get("error") {
        return Err(err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("API Error")
            .to_string());
    }

    let summary = val["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("无法生成总结")
        .to_string();

    Ok(summary)
}

/// 查询最近 N 天的 token 用量摘要（按天聚合）。
///
/// 返回按日期降序排列的每日 input/output tokens 与请求次数。
/// 用于前端用量面板展示（如"过去 7 天 / 30 天用量"）。
#[tauri::command]
pub fn get_usage_summary(
    db_state: tauri::State<'_, DbState>,
    days: u32,
) -> Result<Vec<UsageSummary>, String> {
    let conn = db_state.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT date(timestamp) as day,
                    SUM(input_tokens) as total_input,
                    SUM(output_tokens) as total_output,
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
                request_count: row.get(3)?,
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
) -> Result<Vec<UsageSummaryByModel>, String> {
    let conn = db_state.0.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT model_id,
                    SUM(input_tokens) as total_input,
                    SUM(output_tokens) as total_output,
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
                request_count: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut summaries = Vec::new();
    for row in rows {
        summaries.push(row.map_err(|e| e.to_string())?);
    }
    Ok(summaries)
}

#[tauri::command]
pub async fn append_message(
    state: tauri::State<'_, DbState>,
    topic_id: String,
    message: Message,
) -> Result<(), String> {
    let conn = (*state).0.lock().unwrap();
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
          agent_steps_json, interim_content, agent_start_time)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
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
        ],
    ).map_err(|e| e.to_string())?;
    sync_message_attachments(&conn, &message_id, message.display_files.as_ref())?;
    Ok(())
}

#[tauri::command]
pub async fn delete_topic_message(
    state: tauri::State<'_, DbState>,
    topic_id: String,
    message_id: String,
) -> Result<(), String> {
    let conn = (*state).0.lock().unwrap();
    conn.execute(
        "DELETE FROM messages WHERE id = ?1 AND topic_id = ?2",
        params![message_id, topic_id],
    ).map_err(|e| e.to_string())?;
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
        "好的，标题是：", "好的，标题是:", "好的，标题：", "好的，标题:",
        "好的：", "好的:", "好的，", "好的,",
        "标题是：", "标题是:", "标题：", "标题:",
        "Title:", "Title：", "title:", "title：",
        "以下是", "以下为", "下面给出", "给你一个",
        "Here is the title:", "Here is the title：",
        "The title is:", "The title is：",
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
    messages: Vec<Message>,
) -> Result<String, String> {
    // SSRF 防护
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    if messages.is_empty() {
        return Err("生成标题需要至少一条消息".to_string());
    }

    let client = non_streaming_http_client();

    // 消息顺序遵循 LLM 约定：system 指令 → 对话上下文 → user 明确任务请求
    // 将 system 放最前、user 任务请求放最后，能显著提升小模型 / 本地模型的格式遵循度
    let mut messages_for_api: Vec<serde_json::Value> = vec![json!({
        "role": "system",
        "content": "你是一个话题标题生成助手，擅长用最少的字数精准概括对话核心内容。"
    })];

    // 注入对话历史：多模态 content 只取 text 部分，避免图片 base64 干扰生成
    for m in &messages {
        let text = extract_text_content(&m.content);
        if text.trim().is_empty() {
            continue;
        }
        messages_for_api.push(json!({ "role": m.role, "content": text }));
    }

    // 末尾追加明确的 user 任务请求，作为模型"应输出什么"的最终信号
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
        // 200 token 足够覆盖"标题：xxx + 解释"等冗余输出；
        // 我们会在 Rust 侧再截断到 20 字符
        "max_tokens": 200,
        "temperature": 0.0
    });

    // URL 处理：去掉末尾斜杠与可能的 /chat/completions 后缀
    let base_url = api_url
        .trim_end_matches('/')
        .replace("/chat/completions", "");
    let endpoint = format!("{}/chat/completions", base_url);

    let res = tokio::time::timeout(
        std::time::Duration::from_secs(45),
        client
            .post(endpoint)
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&body)
            .send(),
    )
    .await
    .map_err(|_| "标题生成请求超时（45s）".to_string())?
    .map_err(|e| e.to_string())?;

    let val: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;

    if let Some(err) = val.get("error") {
        return Err(err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("API Error")
            .to_string());
    }

    let raw = val["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_string();

    let cleaned = clean_topic_title(&raw);

    if cleaned.is_empty() {
        // 附带诊断信息：模型 / finish_reason / 原始长度
        let finish = val["choices"][0]["finish_reason"]
            .as_str()
            .unwrap_or("unknown");
        return Err(format!(
            "模型 {} 返回的标题为空 (finish_reason={}, raw_len={})",
            model,
            finish,
            raw.len()
        ));
    }

    // 长度限制：超过 20 字符截断（按字符而非字节，避免中文乱码）
    let truncated: String = if cleaned.chars().count() > 20 {
        cleaned.chars().take(20).collect()
    } else {
        cleaned
    };

    Ok(truncated)
}
