pub mod btw;

use crate::commands::attachment::sync_message_attachments;
use crate::core::events as ev;
use crate::core::models::*;
use crate::core::permission::{self, PermissionAction};
use crate::core::state::DbState;
use crate::core::state::StreamManager;
use crate::core::subagent;
use crate::plugins::mcp::PendingApprovals;
use crate::utils::diff::compute_diff_preview;
use crate::utils::file_tools;
use crate::utils::git_tools;
use crate::utils::lsp_agent_tools;
use crate::utils::lsp_tools;
use crate::utils::shell_tools;
use crate::utils::truncate::{truncate_chars_head, truncate_chars_head_tail, truncate_tool_result};
use crate::utils::web_tools;
use base64::engine::Engine as _;
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt; // 用于处理流式数据
use serde::Serialize;
use serde_json::json;
use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Window}; // Emitter 用于从后端向前端推送事件
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

pub(crate) mod budget;
pub(crate) mod checkpoints;
pub(crate) mod client;
pub(crate) mod handoff;
pub(crate) mod messages;
pub(crate) mod subagents;
pub(crate) mod titles;
pub(crate) mod tool_dispatch;
pub(crate) mod tool_specs;
pub(crate) mod usage;

pub(crate) use budget::*;
pub(crate) use checkpoints::*;
pub(crate) use client::*;
pub(crate) use handoff::*;
pub(crate) use messages::*;
pub(crate) use subagents::*;
pub(crate) use titles::*;
pub(crate) use tool_dispatch::*;
pub(crate) use tool_specs::*;
pub(crate) use usage::*;
/// Agent 循环的轮数与上下文预算（主循环按模型窗口推导；子 Agent 用保守常量，
/// 但轮数上限尊重用户显式配置）。
///
/// 历史问题：主循环与子 Agent 循环各自内联预算，子 Agent 硬编码 100 轮/45k token
/// 且完全忽略 `AppConfig.max_tool_rounds`——两套循环行为漂移。
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentLimits {
    /// 单次运行的模型调用轮数上限（防失控；命中后优雅收尾并输出进度摘要）
    pub max_rounds: u32,
    /// 上下文 token 预算：超过则触发压缩
    pub context_budget: usize,
    /// 压缩后保留区 token 预算
    pub keep_recent_tokens: usize,
}

impl AgentLimits {
    /// 子 Agent 的预算：默认 100 轮失控保护（对齐"不设低轮数上限、靠上下文管理兜底"）；
    /// 用户在设置中显式指定 max_tool_rounds 时以其为准。
    /// 上下文安全阀取保守常量（无需模型窗口信息，对 ≥64k 窗口的模型均安全）。
    pub(crate) fn for_subagent(cfg: &AppConfig) -> Self {
        /// 失控保护轮数（子 Agent 默认上限）
        const GUARD_ROUNDS: u32 = 100;
        /// 保守上下文预算（token）
        const CONTEXT_BUDGET: usize = 45_000;
        Self {
            max_rounds: cfg.max_tool_rounds.unwrap_or(GUARD_ROUNDS),
            context_budget: CONTEXT_BUDGET,
            keep_recent_tokens: (CONTEXT_BUDGET / 2).clamp(4_000, 20_000),
        }
    }
}

/// 单轮流式调用后的累积结果。
pub(crate) struct RoundResult {
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
pub(crate) struct ToolCallAccum {
    id: String,
    name: String,
    arguments: String,
}

// ToolCallPayload 已下沉到 core::models（chat 域）——plugins/provider 需要它；
// 此处再导出以保持 crate::commands::llm::ToolCallPayload 路径不变。
pub use crate::core::models::ToolCallPayload;

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
#[allow(
    clippy::too_many_arguments,
    reason = "stream orchestration keeps request and event context explicit"
)]
pub(crate) async fn stream_one_round(
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
    max_retries: u32,
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

    // 统一请求入口（P2.4 带前置阶段重试）：Anthropic 原生协议自动适配；
    // 传输错误与 429/5xx 在未发射任何内容前按退避重试，流中段错误不重试
    let (response, is_anthropic_request) = crate::plugins::llm::post_chat_completion_with_retry(
        client,
        &api_url,
        api_key,
        &body,
        max_retries,
    )
    .await?;

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
                                ev::LLM_CHUNK,
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
                                                ev::LLM_CHUNK,
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
                                    ev::LLM_REASONING,
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
                                    ev::LLM_TOOL_CALL,
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
                ev::LLM_TOOL_CALL,
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
    // 诊断日志（仅 debug）：打印 messages 每个元素的 role 和关键字段是否存在。
    // 注意保持 debug 级别——生产默认 warn，此处对话内容相关日志不应进入生产日志。
    for (i, m) in messages.iter().enumerate() {
        tracing::debug!(
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
    // 1. 生成任务 Key（结构化元组，避免 "-" 拼接歧义）
    let task_key = (assistant_id.clone(), topic_id.clone());

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

    // 防御性校验（仅调试构建）：检查 tool_calls 与 tool 响应是否匹配。
    // 该扫描对每条消息做 JSON 检查仅为打日志用，发布构建不付出此开销。
    #[cfg(debug_assertions)]
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
            crate::core::models::default_model_call_retries(),
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
                    ev::LLM_CHUNK,
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
                    ev::LLM_CHUNK,
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
    let task_key = (assistant_id.clone(), topic_id.clone());

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
pub(crate) async fn execute_builtin_tool(
    app: &AppHandle,
    tool_name: &str,
    arguments: &serde_json::Value,
    project_id: Option<&str>,
    agent_mode: &AgentMode,
    token: &CancellationToken,
) -> Result<ToolResult, String> {
    let project_root = file_tools::resolve_project_root(app, project_id)?;

    // P3.2 fail-closed：沙箱不可用（无 bwrap/sandbox-exec）时，自主模式按配置
    // 直接拒绝执行命令——此前为 fail-open 裸跑。Normal/Plan 模式继续走权限审批
    //（Ask），审批原因中会附带沙箱不可用警示（见下方 Ask 分支）。
    if tool_name == "execute_command"
        && matches!(agent_mode, AgentMode::Auto | AgentMode::Workflow)
        && !crate::utils::sandbox::sandbox_available()
        && crate::core::config::load_app_config(app)
            .unwrap_or_default()
            .sandbox_required
    {
        return Err("沙箱不可用，已拒绝执行命令（fail-closed）。请安装 bubblewrap（Linux）或确认 sandbox-exec 存在（macOS）后重试；或在应用配置中将 sandboxRequired 设为 false 以回退无沙箱执行（不推荐）。".into());
    }

    // 权限检查（与 execute_tool_call 保持一致，server_id 沿用 crate::core::ids::BUILTIN_FS_SERVER_ID 以兼容已有规则）
    if *agent_mode != AgentMode::Off {
        let custom_rules = permission::load_permissions(Some(&project_root)).rules;
        let action = permission::check_permission(
            tool_name,
            crate::core::ids::BUILTIN_FS_SERVER_ID,
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
                // 网络声明与沙箱可用性标注（P3.1/P3.2）：审批信息完整呈现风险面
                if tool_name == "execute_command" {
                    if arguments["network"].as_bool().unwrap_or(false) {
                        reason.push_str("\n\n⚠ 此命令声明需要网络访问");
                    }
                    if !crate::utils::sandbox::sandbox_available() {
                        reason.push_str("\n\n⚠ 沙箱不可用：此命令将在无 OS 级沙箱的环境执行");
                    }
                }
                // 文件修改工具：预计算 diff 预览
                let (preview_diff, file_path) =
                    compute_diff_preview(tool_name, arguments, &project_root);
                let approval_fut = crate::commands::mcp::request_tool_approval(
                    app,
                    pending.inner(),
                    crate::core::ids::BUILTIN_FS_SERVER_ID,
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
        // 网络出口管控（P3.1）：模型显式声明，默认 false = 沙箱内无网络
        let allow_network = arguments["network"].as_bool().unwrap_or(false);
        // P3.3 Windows AppContainer：配置开启时传入工具链 R+X 目录走 OS 级沙箱
        #[cfg(windows)]
        let ac_dirs: Option<Vec<String>> = {
            let ac_cfg = crate::core::config::load_app_config(app).unwrap_or_default();
            if ac_cfg.sandbox_windows == "appcontainer" {
                Some(ac_cfg.sandbox_windows_toolchain_dirs.clone())
            } else {
                None
            }
        };
        #[cfg(not(windows))]
        let ac_dirs: Option<Vec<String>> = None;
        let root = project_root.clone();
        Ok(tokio::task::spawn_blocking(move || {
            shell_tools::execute_command(&command, &root, timeout, allow_network, ac_dirs)
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
    } else if crate::services::memory::tools::MEMORY_TOOL_NAMES.contains(&tool_name) {
        // 记忆工具名清单由 services/memory/tools.rs 统一维护（含 search_code）；
        // 禁止在此再写字面值清单，防止派发与规格漂移（history: search_code 曾因此不可用）。
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
    } else if matches!(
        tool_name,
        "read_file"
            | "write_file"
            | "replace_in_file"
            | "search_files"
            | "search_content"
            | "list_directory"
            | "delete_file"
            | "make_directory"
    ) {
        let tool_name_c = tool_name.to_string();
        let arguments_c = arguments.clone();
        let root_c = project_root.clone();
        Ok(tokio::task::spawn_blocking(move || {
            file_tools::execute_file_tool(&tool_name_c, &arguments_c, &root_c)
        })
        .await
        .unwrap_or_else(|_| file_tools::tool_err("文件工具执行线程 panic")))
    } else {
        // 未知工具必须显式报错。历史上最后的 else 会把任何未匹配的工具名交给
        // file_tools 兜底，使模型收到语义模糊的结果（search_code 曾因此不可用）。
        Ok(file_tools::tool_err(&format!("未知工具: {tool_name}")))
    }
}

/// Helper: check if a directory is a git repo
fn is_git_repo(dir: &str) -> bool {
    std::path::Path::new(dir).join(".git").exists()
}

/// 带自动重试的工具执行包装器（仅对 MCP 网络调用使用）。
pub(crate) async fn execute_tool_with_retry<F, Fut>(
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

/// 主 Agent 一轮自驱循环的运行上下文（收敛原 15 个扁平业务参数）。
///
/// IPC 兼容面：Tauri 命令仍保留扁平参数（旧前端调用不动），命令体仅为薄包装，
/// 立即构造 TurnContext 交给实现函数。
pub(crate) struct TurnContext {
    pub api_url: String,
    pub api_key: String,
    pub model: String,
    pub assistant_id: String,
    pub topic_id: String,
    pub messages: Vec<Message>,
    pub agent_mode: AgentMode,
    pub project_id: Option<String>,
    pub web_search_enabled: bool,
    pub profile_model_overrides: Vec<ProfileModelOverride>,
    pub custom_subagent_profiles: Vec<crate::core::models::CustomSubagentProfile>,
    pub locale: Option<String>,
    pub context_window: Option<u32>,
    pub skills: Option<Vec<SkillConfig>>,
    /// 助手勾选的 MCP server id 列表（opt-in，空 = 无 MCP 工具）
    pub mcp_server_ids: Vec<String>,
    /// P4.4：恢复指定回合的检查点（崩溃后续跑）；None = 正常新回合
    pub resume_turn_id: Option<String>,
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
/// - `mcp_server_ids`：助手勾选的 MCP server id（opt-in）。Agent 模式下注入已连接
///   server 的工具；工具名冲突时重命名为 mcp__<server>__<tool>；未信任 server 的
///   工具在 Auto/Workflow 模式下仍逐次审批（见 mcp::execute_tool_call）
/// - `agent_mode`：Agent 执行模式，影响权限规则与工具注入（Plan: 研究工具 / Normal/Auto/Workflow: 编排工具 / Off: 无工具）
/// - `project_id`：项目 id（用于解析项目级权限规则）
/// - `locale`：可选界面语言；旧调用缺失时兼容回退到 `zh-CN`
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri IPC compatibility: the flat signature is the existing public command surface"
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
    mcp_server_ids: Vec<String>,
    agent_mode: AgentMode,
    project_id: Option<String>,
    web_search_enabled: bool,
    profile_model_overrides: Vec<ProfileModelOverride>,
    custom_subagent_profiles: Vec<crate::core::models::CustomSubagentProfile>,
    locale: Option<String>,
    context_window: Option<u32>,
    skills: Option<Vec<SkillConfig>>,
    resume_turn_id: Option<String>,
) -> Result<(), String> {
    let ctx = TurnContext {
        api_url,
        api_key,
        model,
        assistant_id,
        topic_id,
        messages,
        agent_mode,
        project_id,
        web_search_enabled,
        profile_model_overrides,
        custom_subagent_profiles,
        locale,
        context_window,
        skills,
        mcp_server_ids,
        resume_turn_id,
    };
    run_agent_turn_inner(app, window, db_state, stream_mgr, ctx).await
}

/// 停止指定的后台子智能体（delegate_task wait=false 启动的 fire-and-forget 子智能体）。
///
/// # 参数
/// - `subagent_id`：后台子智能体 ID（启动确认结果中返回）
///
/// # 返回
/// true = 已发出取消信号；false = 未找到（已完成或不存在，幂等成功）。
#[tauri::command]
pub fn stop_subagent(app: AppHandle, subagent_id: String) -> Result<bool, String> {
    let handles = app.state::<crate::core::state::SubagentHandles>();
    // Ref 守卫必须在本语句内释放（match 作为尾表达式会把临时值活到块尾，触发 E0597）
    let found = match handles.0.get(&subagent_id) {
        Some(entry) => {
            entry.1.cancel();
            tracing::info!("[subagent:{subagent_id}] 用户请求停止后台子智能体");
            true
        }
        None => false,
    };
    Ok(found)
}

/// 主 Agent 一轮自驱循环的实现（参数已收敛为 TurnContext）。
pub(crate) async fn run_agent_turn_inner(
    app: AppHandle,
    window: Window,
    db_state: tauri::State<'_, DbState>,
    stream_mgr: tauri::State<'_, StreamManager>,
    ctx: TurnContext,
) -> Result<(), String> {
    let TurnContext {
        api_url,
        api_key,
        model,
        assistant_id,
        topic_id,
        messages,
        agent_mode,
        project_id,
        web_search_enabled,
        profile_model_overrides,
        custom_subagent_profiles,
        locale,
        context_window,
        skills,
        mcp_server_ids,
        resume_turn_id,
    } = ctx;
    // 诊断日志（仅 debug）：打印 messages 每个元素的 role 和关键字段是否存在。
    // 注意保持 debug 级别——生产默认 warn，此处对话内容相关日志不应进入生产日志。
    for (i, m) in messages.iter().enumerate() {
        tracing::debug!(
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
    let task_key = (assistant_id.clone(), topic_id.clone());

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
    // P4.4 检查点恢复：替换为崩溃前最后一轮的完整上下文快照（继续跑而不是从头跑）。
    // turn_id 贯穿本回合：恢复时沿用原 turn_id（快照续写），新回合生成新 id。
    let turn_id = resume_turn_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if let Some(resume_id) = &resume_turn_id {
        let conn = db_state.0.lock();
        match load_checkpoint_messages(&conn, &topic_id, resume_id) {
            Some(checkpoint_msgs) if !checkpoint_msgs.is_empty() => {
                tracing::info!(
                    "[checkpoint] 从检查点恢复回合 {resume_id}（{} 条消息）",
                    checkpoint_msgs.len()
                );
                messages_for_api = checkpoint_msgs;
            }
            _ => {
                tracing::warn!("[checkpoint] 未找到回合 {resume_id} 的活跃检查点，按新回合执行");
            }
        }
    }
    // 防御性校验（仅调试构建）：发布构建不付出全量消息扫描的日志开销
    #[cfg(debug_assertions)]
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
    let mcp_server_ids_c = mcp_server_ids.clone();

    let handle = tokio::spawn(async move {
        // 解包 Skills（前端仅在 Agent 模式下传递；聊天模式为空）
        let skills = skills.unwrap_or_default();
        // 加载应用配置（含自动重试设置）
        // 应用配置（含自动重试/并发上限/记忆开关）；读取失败回退默认值
        let app_config = crate::core::config::load_app_config(&app_c).unwrap_or_default();

        // 子 Agent 并发上限控制
        let max_concurrent = app_config.max_concurrent_subagents.unwrap_or(5).max(1) as usize;
        let subagent_semaphore = Arc::new(Semaphore::new(max_concurrent));

        let client = streaming_http_client();

        // 工具选择：根据 Agent 模式注入不同工具集
        // - Plan: 仅研究工具（think + project_map + web）
        // - Normal/Auto/Workflow: 仅编排工具（delegate_task + create_workflow + think + web + project_map）
        // - 对话 + 联网: 仅 web 工具
        // - 纯对话: 无工具
        // MCP 工具回接（P2.2）：Agent 模式下注入助手勾选 server 的已连接工具。
        // 仅拉取已连接 server（未连接的跳过并沿用既有警告语义）；工具名冲突
        // （内置工具或其他 server 占用）在合并时重命名，见 build_main_tool_set。
        let mcp_tools: Option<crate::core::models::AssistantTools> =
            if agent_mode != AgentMode::Off && !mcp_server_ids_c.is_empty() {
                let mgr = app_c.state::<crate::plugins::mcp::McpServerManager>();
                let mcp_state = app_c.state::<crate::plugins::mcp::McpServerState>();
                crate::commands::mcp::list_mcp_tools_for_assistant_inner(
                    &app_c,
                    mgr.inner(),
                    mcp_state.inner(),
                    mcp_server_ids_c.clone(),
                    project_id_c.clone(),
                )
                .await
                .ok()
            } else {
                None
            };

        // 工具集装配（模式矩阵 + MCP/记忆/Skill/归档注入）——已抽取为 build_main_tool_set
        let tool_set = build_main_tool_set(
            &app_c,
            &agent_mode,
            web_search_enabled,
            project_id_c.as_deref(),
            &skills,
            app_config.memory_enabled,
            mcp_tools.as_ref(),
            &app_config.main_agent_tool_access,
        );
        let tools = tool_set.tools;
        let tool_server_map = tool_set.tool_server_map;
        let memory_effective = tool_set.memory_effective;
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
                            // 项目级嵌入配置（与写入侧一致）+ 维度守卫；随后混合检索 → 项目级重排 → 预算裁剪
                            let query_vec = crate::services::memory::retrieve::embed_query(
                                &app_c, &store, &query,
                            )
                            .await;
                            if let Some(block) = crate::services::memory::retrieve::inject_block(
                                &app_c, &store, &query, query_vec, budget,
                            )
                            .await
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
        // 上回合交接回注（P1.6）：若上一回合未完成（轮数上限/错误/截断），
        // 消费其 handoff 文档注入系统消息——新回合直接拿到已完成内容与涉及
        // 文件，而非只靠用户转述。consume-once：读取后删除，不重复注入。
        if let Some(handoff) = consume_handoff_doc(&app_c, &assistant_id_c, &topic_id_c) {
            // 插入点不得超过最后一条消息（user 消息必须保持在末尾）
            let insert_at = 2.min(messages_for_api.len().saturating_sub(1));
            messages_for_api.insert(
                insert_at,
                serde_json::json!({
                    "role": "system",
                    "content": format!("[上回合交接] 上一回合任务未完成，以下是交接文档（含已完成内容与涉及文件），请据此继续：
{}", handoff),
                }),
            );
            tracing::info!(
                "[handoff] 已回注上回合交接文档 ({} 字符)",
                handoff.chars().count()
            );
        }
        // 上下文预算追踪：token 级、模型感知（阈值/保留预算在任务启动前按窗口推导）
        // 可配置的单次最大工具调用轮数（默认 25，最低 1）
        let max_rounds = app_config
            .max_tool_rounds
            .unwrap_or(crate::core::models::DEFAULT_MAX_TOOL_ROUNDS)
            .max(1);
        // Workflow 模式下提醒调用 create_workflow 的最大次数（有限次，避免无限循环）
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

            // 达到最大工具调用轮数 → 优雅收尾（不 abort，产出提示后自然结束）
            if round > max_rounds {
                incomplete_stop = true;
                let note = "\n\n⚠️ 已达本轮最大工具调用轮数，任务未完成。如需继续，请再次发送。";
                accumulated_content.push_str(note);
                let _ = window.emit(
                    ev::LLM_CHUNK,
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
                ev::LLM_ROUND_START,
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
                app_config.model_call_retries,
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
                    // Workflow 模式：提醒次数有限（max_workflow_reminders），避免无限续环
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
                        ev::LLM_CHUNK,
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

            // 执行工具调用：三阶段
            // 阶段 1 — 收集并发 future（delegate_task(s) 委派 + 只读内置工具），
            //          变更类工具（文件写/命令/Git 写/MCP——副作用未知）按原始顺序串行执行
            // 阶段 2 — FuturesUnordered 并发等待全部 future，完成即发射结果
            // 阶段 3 — 按原始 index 顺序发射串行结果 + 回填全部 role:tool 消息
            //（对齐 Claude Code / Codex CLI 的并行工具调用语义；回填顺序保持索引序，API 语义不变）

            // 用于记录每个工具的结果（按原始 index 排序）
            #[derive(Default, Clone)]
            struct ToolExecResult {
                content_text: String,
                result_value: serde_json::Value,
                is_error: bool,
            }
            let mut exec_results: Vec<Option<ToolExecResult>> =
                vec![None; round_result.tool_calls.len()];

            // 收集可并发执行的 future（子 Agent 委派 + 只读内置工具）及对应 index
            let mut parallel_futures: Vec<(
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

                if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                    && tc.name == "delegate_task"
                {
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
                        // 子令牌：父取消级联取消；stop_subagent 可按 ID 单独取消此后台子智能体
                        let token_clone = token_inner.child_token();
                        let token_for_registry = token_clone.clone();
                        let locale_clone = locale_c.clone();
                        let mcp_ids_clone = mcp_server_ids_c.clone();
                        let sem_clone = subagent_semaphore.clone();
                        let subagent_id = uuid::Uuid::new_v4().to_string();
                        let sid = subagent_id.clone();
                        let app_c2 = app_c.clone();
                        let jh = tokio::spawn(async move {
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
                                &mcp_ids_clone,
                                &locale_clone,
                            )
                            .await;
                            // Remove handle from registry on completion
                            if let Some(handles) =
                                app_c2.try_state::<crate::core::state::SubagentHandles>()
                            {
                                handles.0.remove(&sid);
                            }
                            // 后台子智能体的结果不回传主 Agent（fire-and-forget），
                            // 但错误必须可见（此前静默丢弃，失败无从排查）。
                            if let Err(e) = &result {
                                tracing::error!("[subagent:{sid}] 后台子智能体失败: {e}");
                            }
                        });
                        // 登记句柄与令牌：stop_subagent 可按 ID 单独取消。
                        // （此前为死注册表：句柄从不插入，完成时只删除不存在的键）
                        if let Some(handles) =
                            app_c.try_state::<crate::core::state::SubagentHandles>()
                        {
                            handles
                                .0
                                .insert(subagent_id.clone(), (jh, token_for_registry));
                        }
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
                        let mcp_ids_clone = mcp_server_ids_c.clone();
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
                                &mcp_ids_clone,
                                &locale_clone,
                            )
                            .await
                        });
                        parallel_futures.push((i, fut));
                    }
                } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                    && tc.name == "delegate_tasks"
                {
                    // 批量并行：收集一个 future，放入 parallel_futures
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
                    let mcp_ids_clone = mcp_server_ids_c.clone();

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
                            &mcp_ids_clone,
                            semaphore_clone,
                            &locale_clone,
                        )
                        .await
                    });
                    parallel_futures.push((i, fut));
                } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
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
                                    &mcp_server_ids_c,
                                    &token_inner,
                                    &locale_c,
                                    &agent_mode,
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
                    let tool_result = if server_id.as_deref()
                        == Some(crate::core::ids::BUILTIN_SERVER_ID)
                        && tc.name == "read_artifact"
                    {
                        execute_artifact_read(
                            &app_c,
                            args_val["artifact_id"].as_str().unwrap_or(""),
                        )
                    } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                        && tc.name == "read_skill"
                    {
                        execute_skill_read(&skills, args_val["name"].as_str().unwrap_or(""))
                    } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                        && app_config.parallel_tool_execution
                        && is_parallel_safe(&tc.name)
                    {
                        // 只读内置工具：并发执行（对齐 Claude Code / Codex 的并行工具
                        // 调用语义）。收集为 future 与子 Agent 委派共用阶段 2 的
                        // FuturesUnordered，完成即发射；变更类工具不受影响（仍按原始
                        // 顺序串行），role:tool 回填保持索引序，API 语义不变。
                        let app_c2 = app_c.clone();
                        let tn = tc.name.clone();
                        let args_val_c = args_val.clone();
                        let pid = project_id_c.clone();
                        let mode = agent_mode.clone();
                        let tok = token_inner.clone();
                        let fut = Box::pin(async move {
                            execute_builtin_tool(
                                &app_c2,
                                &tn,
                                &args_val_c,
                                pid.as_deref(),
                                &mode,
                                &tok,
                            )
                            .await
                        });
                        parallel_futures.push((i, fut));
                        continue;
                    } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID) {
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
                        ev::LLM_TOOL_RESULT,
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

            // 阶段 2 — 并发执行子 Agent 委派与只读内置工具（FuturesUnordered）
            if !parallel_futures.is_empty() {
                let mut unordered = FuturesUnordered::new();
                for (i, fut) in parallel_futures {
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
                        ev::LLM_TOOL_RESULT,
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
                        ev::LLM_TOOL_RESULT,
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
                            ev::LLM_COMPRESSION,
                            json!({
                                "round": round,
                                "result": "ok",
                                "estimated_tokens": estimated_tokens,
                                "keep_recent_tokens": keep_recent_tokens,
                            }),
                        );
                    }
                    Err(e) => {
                        let _ = window.emit(ev::LLM_COMPRESSION_FAILED, json!({"error": e}));
                    }
                }
                messages_for_api = msgs;
            }
        }

        // P4.4 检查点：本轮上下文已定（工具回填 + 压缩判定后），持久化快照。
        // 崩溃后 run_agent_turn(resume_turn_id) 从最新 active 快照续跑；
        // epilogue 把本 turn 标记终态——active 残留即"可恢复"信号。
        {
            let conn = app_c.state::<crate::core::state::DbState>();
            save_checkpoint(
                &conn.0.lock(),
                &topic_id_c,
                &turn_id,
                round,
                &messages_for_api,
            );
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
                        ev::LLM_HANDOFF,
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
                        ev::LLM_CHUNK,
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
        // P4.4：回合到达 epilogue = 非崩溃终止 → 检查点标记终态；
        // 崩溃残留（status 仍为 active）才是"可恢复"信号。
        {
            let status = if was_cancelled {
                "cancelled"
            } else {
                "completed"
            };
            let conn = app_c.state::<crate::core::state::DbState>();
            finalize_checkpoints(&conn.0.lock(), &topic_id_c, &turn_id, status);
        }
        let error_payload = if was_cancelled {
            None
        } else {
            final_error.clone()
        };
        let _ = window.emit(
            ev::LLM_CHUNK,
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

/// 上下文自动压缩：将早期消息压缩为摘要，按 token 预算保留最近消息。
/// 用于 Agent 循环中避免超出模型上下文窗口限制。
/// - 前置的 system 消息块（稳定提示词前缀：locale / 知识 / 工作流指令等）永不压缩；
/// - 保留区按 keep_recent_tokens token 预算从尾部累计，并回退到完整轮次边界（不切断 role:tool）；
/// - 摘要输入有界（最多约 30k 字符，最近优先），并固定带上首条 user 消息（原始任务）。
#[allow(
    clippy::too_many_arguments,
    reason = "context compression reuses the active stream request context"
)]
pub(crate) async fn compress_context(
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
            let placeholder = archive_early_messages(window.app_handle(), &early, model)?;
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

    let _ = window.emit(ev::LLM_COMPRESSION, json!({
        "assistant_id": assistant_id,
        "topic_id": topic_id,
        "source": source,
        "summary": summary,
        "compressed_chars": early.iter().map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0)).sum::<usize>(),
    }));

    Ok(())
}
