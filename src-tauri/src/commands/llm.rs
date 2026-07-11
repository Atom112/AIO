use crate::core::permission::{self, PermissionAction};
use crate::core::state::DbState;
use crate::core::state::PendingApprovals;
use crate::commands::attachment::sync_message_attachments;
use crate::core::state::McpServerState;
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
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use tauri::{AppHandle, Emitter, Manager, Window}; // Emitter 用于从后端向前端推送事件
use tokio_util::sync::CancellationToken;

/// 构造带超时的 reqwest 客户端（防止 DoS）
///
/// 注意：流式请求**不设置总超时**（仅保留 connect_timeout）。
/// 旧的 `.timeout(60s)` 是从连接到响应体结束的总时长，会杀死任何超过 60s 的流
/// （长推理模型 / 慢本地服务器），导致工具调用中途断流。
fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
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
        // 取消检查：select! 让 cancelled 与 stream.next 竞争
        let next = tokio::select! {
            _ = token.cancelled() => return Err("cancelled".to_string()),
            item = stream.next() => item,
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
                    if let Some(reasoning) = val["choices"][0]["delta"]["reasoning_content"]
                        .as_str()
                        .or_else(|| val["choices"][0]["delta"]["reasoning"].as_str())
                    {
                        if !reasoning.is_empty() {
                            reasoning_buf.push_str(reasoning);
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
    if need_emit {
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

    // 4. 创建异步任务执行请求
    let handle = tokio::spawn(async move {
        let client = http_client();
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
        )
        .await;

        // 收尾：无论成功/取消/错误都 emit terminal done，保证前端 isThinking 必复位
        match result {
            Ok(round) => {
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
    // 构造模型获取地址，通常是基础 URL 后接 /models
    let mut base_url = api_url.trim_end_matches('/').to_string();
    if base_url.ends_with("/chat/completions") {
        base_url = base_url.replace("/chat/completions", "");
    }
    let final_url = format!("{}/models", base_url);

    let client = http_client();
    let response = client
        .get(&final_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
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
                        if let Some(risk) = shell_tools::check_dangerous_command(cmd) {
                            reason = format!("⚠️ {risk}\n\n命令: {cmd}\n\n工具 '{}' 需要您的确认才能执行", tool_name);
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
        let command = arguments["command"].as_str().unwrap_or("");
        let timeout = arguments["timeout"].as_u64();
        Ok(shell_tools::execute_command(command, &project_root, timeout))
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
        Ok(git_tools::execute_git_tool(tool_name, arguments, &project_root))
    } else {
        Ok(file_tools::execute_file_tool(tool_name, arguments, &project_root))
    }
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
) -> Result<(), String> {
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

    let handle = tokio::spawn(async move {
        let client = http_client();

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
        // 跨轮累计 token 用量
        let mut total_input_tokens: u32 = 0;
        let mut total_output_tokens: u32 = 0;

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
                break 'outer;
            }

            // 执行每个工具调用（按 index 升序），回填 role:tool 消息
            for tc in &round_result.tool_calls {
                if token_inner.is_cancelled() {
                    was_cancelled = true;
                    break 'outer;
                }
                let server_id = tool_server_map.get(&tc.name).cloned();
                let args_val: serde_json::Value =
                    serde_json::from_str(&tc.arguments).unwrap_or(json!({}));
                let tool_result = if server_id.as_deref() == Some("__builtin__") {
                    // 内置文件工具：in-process 直接执行，含权限检查和审批
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

                // emit 工具结果，前端据此更新气泡 + 追加 role:tool 消息
                let _ = window.emit(
                    "llm-tool-result",
                    ToolResultPayload {
                        assistant_id: assistant_id_c.clone(),
                        topic_id: topic_id_c.clone(),
                        tool_call_id: tc.id.clone(),
                        name: tc.name.clone(),
                        content: content_text.clone(),
                        result: result_value,
                        is_error,
                    },
                );

                // 追加 role:tool 消息到上下文（OpenAI 要求 tool_call_id 配对）
                let mut tool_msg = serde_json::Map::new();
                tool_msg.insert("role".into(), json!("tool"));
                tool_msg.insert("content".into(), json!(content_text));
                tool_msg.insert("tool_call_id".into(), json!(tc.id));
                tool_msg.insert("name".into(), json!(tc.name));
                messages_for_api.push(serde_json::Value::Object(tool_msg));
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
    let client = http_client();

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

    let res = client
        .post(endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
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
          tool_call_id, name, tool_calls_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
    if messages.is_empty() {
        return Err("生成标题需要至少一条消息".to_string());
    }

    let client = http_client();

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

    let res = client
        .post(endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
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
