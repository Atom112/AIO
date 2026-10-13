//! AgentTurnRunner：主智能体单回合自主循环的运行器（阶段4 第1项子目标 4/5：主循环闭包 → AgentTurnRunner）。
//!
//! 历史形态：`run_agent_turn_inner` 内一个 ~1230 行的 `async move` 闭包捕获 24 个自由变量
//! （窗口/应用句柄、取消令牌、流管理器、身份与上下文、模型连接参数、预算……），无结构、
//! 无法单测、每加一个捕获字段就悄悄变长。现在捕获集显式化为 `AgentTurnRunner` 结构体：
//! - mod.rs 构造一次（取代逐行 let-clone 捕获列表），`run()` 产出原 body future；
//! - `run()` 顶部按原名解构（`app: app_c`、`token: token_inner`……），1230 行循环体原样迁入、零改动；
//! - 后续子目标（subagents 与主循环抽公共 agent_loop）以此为收敛面。

use crate::core::events as ev;
use crate::core::models::*;
use crate::core::state::StreamManager;
use crate::ierr;
use crate::utils::file_tools;
use crate::utils::git_tools;
use crate::utils::truncate::{truncate_chars_head, truncate_tool_result};
use futures_util::stream::FuturesUnordered;
use futures_util::StreamExt as _;
use serde_json::json;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Window};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

use super::budget::estimate_context_tokens_offload;
use super::checkpoints::finalize_checkpoints;
use super::client::streaming_http_client;
use super::handoff::{build_extract_transcript, consume_handoff_doc, write_handoff_doc};
use super::subagents::{
    drain_pending_subagent_notices, execute_get_subagent_result, handle_delegate_task,
    handle_delegate_tasks, record_subagent_outcome, register_running_subagent,
};
use super::tasks::{execute_ask_user, execute_task_board_tool};
use super::tool_dispatch::{
    build_main_tool_set, classify_outcome, is_parallel_safe, ToolFailureBreaker,
};
use super::tool_specs::{execute_artifact_read, execute_skill_read};
use super::usage::insert_usage_log;
use super::DelegateContext;
use super::{execute_builtin_tool, execute_tool_with_retry};
use crate::core::tool_names;

/// 主循环运行器：单回合自主循环的全部输入（原 async-move 闭包的捕获集）。
pub(crate) struct AgentTurnRunner {
    pub(crate) window: Window,
    pub(crate) app: AppHandle,
    /// 本回合取消令牌（原 token_inner；父令牌供 StreamManager 注册侧使用）
    pub(crate) token: CancellationToken,
    /// 流任务管理器句柄（原 state_inner）
    pub(crate) streams: StreamManager,
    /// (assistant_id, topic_id) 结构化任务键（原 task_key_inner）
    pub(crate) task_key: (String, String),
    pub(crate) assistant_id: String,
    pub(crate) topic_id: String,
    /// LOOP-1 回合代次号（epilogue 身份注销用；原 turn_id_c）
    pub(crate) turn_gen_id: String,
    /// 本回合 ID（检查点与日志；原 turn_id）
    pub(crate) turn_id: String,
    pub(crate) project_id: Option<String>,
    pub(crate) profile_overrides: Vec<ProfileModelOverride>,
    pub(crate) custom_profiles: Vec<CustomSubagentProfile>,
    pub(crate) mcp_server_ids: Vec<String>,
    pub(crate) locale: String,
    pub(crate) agent_mode: AgentMode,
    pub(crate) skills: Option<Vec<SkillConfig>>,
    pub(crate) messages_for_api: Vec<serde_json::Value>,
    pub(crate) api_url: String,
    pub(crate) api_key: String,
    pub(crate) model: String,
    pub(crate) web_search_enabled: bool,
    pub(crate) keep_recent_tokens: usize,
    pub(crate) max_output_tokens: Option<u32>,
    pub(crate) context_budget: usize,
}

impl AgentTurnRunner {
    /// Runs the autonomous turn loop to completion (epilogue included). The returned
    /// future is exactly the historic body_fut; the caller wraps it in catch_unwind.
    pub(crate) async fn run(self) {
        // 按原局部名解构（1230 行循环体零改动；run_agent_turn_inner 的历史命名保持）
        let Self {
            window,
            app: app_c,
            token: token_inner,
            streams: state_inner,
            task_key: task_key_inner,
            assistant_id: assistant_id_c,
            topic_id: topic_id_c,
            turn_gen_id: turn_id_c,
            turn_id,
            project_id: project_id_c,
            profile_overrides: profile_overrides_c,
            custom_profiles: custom_profiles_c,
            mcp_server_ids: mcp_server_ids_c,
            locale: locale_c,
            agent_mode,
            skills,
            mut messages_for_api,
            api_url,
            api_key,
            model,
            web_search_enabled,
            keep_recent_tokens,
            max_output_tokens,
            context_budget,
        } = self;

        // Unwrap Skills (the frontend passes them only in Agent mode; empty in chat mode)
        let skills = skills.unwrap_or_default();
        // Load the app config (incl. auto-retry, subagent concurrency cap, memory toggle);
        // reads fall back to defaults on failure
        let app_config = crate::core::config::load_app_config(&app_c).unwrap_or_default();

        // Subagent concurrency cap (-1 = unlimited, full-autonomy opt-in; 0 is
        // invalid and clamps to 1; the per-subagent round/context budgets and the
        // stop button remain the backstops)
        let max_concurrent = match app_config.max_concurrent_subagents {
            Some(n) if n < 0 => usize::MAX,
            Some(0) => 1,
            v => v.unwrap_or(5).max(1) as usize,
        };
        let subagent_semaphore = Arc::new(Semaphore::new(max_concurrent));

        let client = streaming_http_client();

        // Tool selection: inject a different tool set per agent mode
        // - Plan: research tools only (think + project_map + web)
        // - Normal/Auto: orchestration tools only (delegate_task + think + web + project_map)
        // - Chat + web search: web tools only
        // - Pure chat: no tools
        // MCP tool pass-through: in Agent mode, inject connected tools from the assistant's selected servers.
        // Only connected servers are pulled (unconnected ones are skipped with the existing warning semantics);
        // tool name conflicts (built-in tools or another server) are renamed at merge time, see build_main_tool_set.
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

        // Tool set assembly (mode matrix + MCP/memory/skill/archive injection), see build_main_tool_set
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

        // 阶段4 第1项（agent_loop 切片4）：共享压缩守卫上下文（预算门控 + 溢出恢复）
        let compress_ctx = super::round_artifacts::CompressCtx {
            window: &window,
            client: &client,
            api_url: &api_url,
            api_key: &api_key,
            model: &model,
            assistant_id: &assistant_id_c,
            topic_id: &topic_id_c,
            token: &token_inner,
            keep_recent_tokens,
            tag: "main",
            locale: &locale_c,
        };

        let mut round: u32 = 0;
        let mut final_error: Option<String> = None;
        let mut was_cancelled = false;
        // Token usage accumulated across rounds (for cost stats)
        let mut total_input_tokens: u32 = 0;
        let mut total_output_tokens: u32 = 0;
        let mut total_cached_tokens: u32 = 0;
        // Prefix-stability regression check: flag that compression just happened, to record cache hits on the first post-compression round
        let mut compressed_this_run: bool = false;
        // Unfinished-task tracking (handoff doc): forced stop (round limit) or error
        let mut incomplete_stop: bool = false;
        let mut last_finish_reason: Option<String> = None;
        // Peak context tokens: input_tokens of the last API call (for the context-window display)
        let mut context_input_tokens: u32 = 0;
        // Final text and generated images accumulated across rounds (sent once at done; idempotently replaces frontend chunk accumulation)
        let mut accumulated_content = String::new();
        let mut all_images: Vec<GeneratedImage> = Vec::new();
        // Per-turn tool circuit breaker (harness finding: environment-blocked
        // retries can burn the whole round budget probing). Threshold is
        // configurable via `toolBreakerThreshold` (0 = default 6).
        let mut tool_breaker = ToolFailureBreaker::new(app_config.tool_breaker_threshold);

        // Project memory auto-injection: retrieve once from the first user message and insert at the stable prefix slot (index=1)
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
                            // Project-level embedding config (same as the write side) + dimension guard; then hybrid retrieval -> project-level rerank -> budget trim
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
        // Previous-turn handoff re-injection: if the last turn ended unfinished (round limit / error / truncation),
        // consume its handoff doc and inject it as a system message so the new turn directly gets the completed work
        // and touched files instead of relying on the user's retelling. Consume-once: deleted after read, never injected twice.
        if let Some(handoff) = consume_handoff_doc(&app_c, &assistant_id_c, &topic_id_c) {
            // The insert point must not go past the last message (the user message must stay at the end)
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
        // 后台子智能体完成通知（push 通道）：本回合开始前已到达终态的后台任务，
        // consume-once 注入单条合并 system 消息（覆盖"主 Agent 空闲期间后台完成"的场景）。
        if let Some(notice) =
            drain_pending_subagent_notices(&app_c, &assistant_id_c, &topic_id_c, &locale_c)
        {
            let insert_at = 2.min(messages_for_api.len().saturating_sub(1));
            messages_for_api.insert(
                insert_at,
                serde_json::json!({
                    "role": "system",
                    "content": notice,
                }),
            );
            tracing::info!(
                "[subagent-notice] 回合开始注入后台子智能体通知 ({} 字符)",
                notice.chars().count()
            );
        }
        // Context budget tracking: token-level and model-aware (threshold/reserve budgets derived from the window before task start)
        // Configurable max tool-call rounds per turn (LOOP-15: default 50 per
        // DEFAULT_MAX_TOOL_ROUNDS — the historic comment claimed 25); -1 = unlimited
        // (full-autonomy opt-in, see AgentLimits::effective_max_rounds)
        let max_rounds = super::effective_max_rounds(
            app_config.max_tool_rounds,
            crate::core::models::DEFAULT_MAX_TOOL_ROUNDS,
        );
        // Max continuation retries when output is truncated (finish_reason="length")
        const MAX_LENGTH_RETRIES: u32 = 2;
        let mut length_retries: u32 = 0;
        // Context overflow auto-recovery attempts: compress then retry
        const MAX_OVERFLOW_RETRIES: u32 = 1;
        let mut overflow_retries: u32 = 0;

        'outer: loop {
            round += 1;
            if token_inner.is_cancelled() {
                was_cancelled = true;
                break;
            }

            // Max tool-call rounds reached -> graceful stop (no abort; emit a note and end)
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

            // Notify frontend: a new round starts; it pushes an empty assistant placeholder
            let _ = window.emit(
                ev::LLM_ROUND_START,
                RoundStartPayload {
                    assistant_id: assistant_id_c.clone(),
                    topic_id: topic_id_c.clone(),
                    round,
                },
            );

            // 后台子智能体完成通知（push 通道，轮内）：本回合运行期间到达终态的后台任务，
            // consume-once 注入为尾部 system 消息（本轮 role:tool 已回填、下轮 LLM 调用前，
            // 位置安全；system 消息可出现在消息流任意位置）。
            if let Some(notice) =
                drain_pending_subagent_notices(&app_c, &assistant_id_c, &topic_id_c, &locale_c)
            {
                messages_for_api.push(serde_json::json!({
                    "role": "system",
                    "content": notice,
                }));
                tracing::info!(
                    "[subagent-notice] 轮内注入后台子智能体通知 ({} 字符)",
                    notice.chars().count()
                );
            }

            // No hard round limit below the cap above: complex tasks may need many rounds of tool
            // calls; the loop ends only when the model stops issuing tool calls (task done) or the user stops it.
            let (round_msgs, round_tools) = (messages_for_api.as_slice(), tools_slice);

            let round_result = super::streaming::stream_one_round(
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
                None, // main conversation: deltas emit llm-chunk / llm-reasoning directly
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
                    // Context overflow auto-recovery: compress earlier messages and retry once so long sessions don't fail outright
                    // (shared gate; on false the failure path below stands)
                    if compress_ctx
                        .try_overflow_recovery(
                            &e,
                            messages_for_api.len(),
                            &mut overflow_retries,
                            MAX_OVERFLOW_RETRIES,
                            &mut messages_for_api,
                        )
                        .await
                    {
                        tracing::warn!("[overflow] 上下文溢出，已压缩并重试: {}", e);
                        continue;
                    }
                    incomplete_stop = true;
                    final_error = Some(e);
                    break 'outer;
                }
            };

            // Accumulate token usage
            total_input_tokens += round_result.input_tokens;
            total_output_tokens += round_result.output_tokens;
            total_cached_tokens += round_result.cached_input_tokens;
            last_finish_reason = round_result.finish_reason.clone();

            // Prefix-stability regression check: cache hit rate on the first round after compression (stable prefix blocks should still hit)
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
            // Record peak context (input_tokens of the last round)
            context_input_tokens = round_result.input_tokens;
            // Per-round usage ping (no content): lets the frontend context bar track the
            // growing prompt across a multi-round turn (mainstream agent harnesses update
            // context per API call, not only at turn end). input/output carry the running
            // turn totals so the mid-turn display matches the final done event.
            let _ = window.emit(
                ev::LLM_CHUNK,
                StreamPayload {
                    assistant_id: assistant_id_c.clone(),
                    topic_id: topic_id_c.clone(),
                    content: String::new(),
                    done: false,
                    error: None,
                    input_tokens: Some(total_input_tokens),
                    output_tokens: Some(total_output_tokens),
                    context_tokens: Some(context_input_tokens),
                    cached_input_tokens: Some(total_cached_tokens),
                    images: None,
                },
            );
            // Accumulate final text and generated images (incl. aio-image markers, returned with done)
            accumulated_content.push_str(&round_result.content);
            all_images.extend(round_result.images.clone());

            // Persist this round's token usage to usage_log
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

            // Append this round's assistant message (incl. tool_calls) to the context.
            // 阶段4 第1项（公共 agent_loop 地基）：与子代理循环共享的构造。
            super::round_artifacts::append_assistant_message(
                &mut messages_for_api,
                &round_result,
                true,
            );

            // Output truncated (finish_reason="length") with no tool calls: append a continuation
            // instruction and retry (bounded) instead of ending mid-reply.
            if super::round_artifacts::try_length_continuation(
                &mut messages_for_api,
                round_result.finish_reason.as_deref(),
                round_result.tool_calls.is_empty(),
                &mut length_retries,
                MAX_LENGTH_RETRIES,
            ) {
                continue;
            }

            // No tool calls -> task complete, end the whole turn
            if round_result.tool_calls.is_empty() {
                break 'outer;
            }

            // Tool-call execution: three phases
            // Phase 1 - collect concurrent futures (delegate_task(s) delegation + read-only built-in tools);
            //           mutating tools (file writes / commands / git writes / MCP - side effects unknown) run serially in original order
            // Phase 2 - await all futures concurrently via FuturesUnordered, emitting each result on completion
            // Phase 3 - emit serial results in original index order + backfill all role:tool messages
            // (semantics follow Claude Code / Codex CLI parallel tool calling; backfill keeps index order, API semantics unchanged)

            // Records each tool's result (ordered by original index)
            #[derive(Default, Clone)]
            struct ToolExecResult {
                content_text: String,
                result_value: serde_json::Value,
                is_error: bool,
            }
            let mut exec_results: Vec<Option<ToolExecResult>> =
                vec![None; round_result.tool_calls.len()];

            // Collect concurrently executable futures (subagent delegation + read-only built-in tools) with their indices
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

                // Circuit breaker: a tool that failed CONSECUTIVELY (environment
                // blocking signature) is disabled for the rest of the turn —
                // short-circuit with an explanation instead of executing again
                // (harness finding: a tester subagent burned all 25 rounds probing).
                if tool_breaker.is_disabled(&tc.name) {
                    let blocked = tool_breaker.blocked_message(&tc.name);
                    exec_results[i] = Some(ToolExecResult {
                        content_text: blocked.clone(),
                        result_value: json!({"error": blocked}),
                        is_error: true,
                    });
                    continue;
                }

                if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                    && tc.name == tool_names::DELEGATE_TASK
                {
                    // Check the wait argument: false = run in the background (fire-and-forget)
                    let wait_for_result = args_val
                        .get("wait")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    if !wait_for_result {
                        // Fire-and-forget: the subagent runs in the background, the main agent continues immediately.
                        // Dual channel: outcome cached for get_subagent_result (pull) + completion notice
                        // queued for system-message injection (push).
                        let args_val_clone = args_val.clone();
                        // Child token: parent cancellation cascades; stop_subagent can cancel this background subagent by ID
                        let token_clone = token_inner.child_token();
                        let token_for_registry = token_clone.clone();
                        let subagent_id = uuid::Uuid::new_v4().to_string();
                        let sid = subagent_id.clone();
                        let app_c2 = app_c.clone();
                        // 登记 Running 状态（get_subagent_result 可立即查询）
                        let bg_profile = args_val_clone["profile"]
                            .as_str()
                            .unwrap_or("general")
                            .to_string();
                        let bg_task =
                            truncate_chars_head(args_val_clone["task"].as_str().unwrap_or(""), 100);
                        register_running_subagent(&app_c, &subagent_id, bg_profile, bg_task);
                        // 阶段4 第1项（子目标 3/5）：继承上下文一次打包，替代 17 项克隆瀑布
                        let bg_ctx = DelegateContext {
                            window: window.clone(),
                            app: app_c.clone(),
                            client: client.clone(),
                            api_url: api_url.clone(),
                            api_key: api_key.clone(),
                            model: model.clone(),
                            parent_assistant_id: assistant_id_c.clone(),
                            parent_topic_id: topic_id_c.clone(),
                            project_id: project_id_c.clone(),
                            agent_mode: agent_mode.clone(),
                            token: token_clone,
                            profile_model_overrides: profile_overrides_c.clone(),
                            custom_profiles: custom_profiles_c.clone(),
                            mcp_server_ids: mcp_server_ids_c.clone(),
                            semaphore: subagent_semaphore.clone(),
                            locale: locale_c.clone(),
                        };
                        // 后台收尾通知所需的归属信息（move 进 spawn）
                        let assistant_id_clone = assistant_id_c.clone();
                        let topic_id_clone = topic_id_c.clone();
                        let locale_clone = locale_c.clone();
                        let jh = tokio::spawn(async move {
                            let result = handle_delegate_task(&bg_ctx, &args_val_clone).await;
                            // Remove handle from registry on completion
                            if let Some(handles) =
                                app_c2.try_state::<crate::core::state::SubagentHandles>()
                            {
                                handles.0.remove(&sid);
                            }
                            // 双通道收尾：终态唯一写入点（Done/Failed/Cancelled）+ 通知入队
                            // + 唤醒 get_subagent_result 等待者；错误同时保留 tracing 可见性。
                            let (profile_name, task_summary) = {
                                let completions =
                                    app_c2.state::<crate::core::state::SubagentCompletions>();
                                completions
                                    .outcomes
                                    .get(&sid)
                                    .map(|e| (e.profile_name.clone(), e.task_summary.clone()))
                                    .unwrap_or_default()
                            };
                            record_subagent_outcome(
                                &app_c2,
                                &assistant_id_clone,
                                &topic_id_clone,
                                &sid,
                                &profile_name,
                                &task_summary,
                                &locale_clone,
                                &result,
                            );
                            if let Err(e) = &result {
                                tracing::error!("[subagent:{sid}] 后台子智能体失败: {e}");
                            }
                        });
                        // Register the handle and token so stop_subagent can cancel this subagent by ID.
                        if let Some(handles) =
                            app_c.try_state::<crate::core::state::SubagentHandles>()
                        {
                            handles
                                .0
                                .insert(subagent_id.clone(), (jh, token_for_registry));
                        }
                        // Immediate result: acknowledge the fire-and-forget
                        let content_text = format!(
                            "子智能体已在后台启动 (ID: {})，完成后将自动通知；可随时用 get_subagent_result 查询",
                            subagent_id
                        );
                        exec_results[i] = Some(ToolExecResult {
                            content_text: content_text.clone(),
                            result_value: json!({"status": "started", "subagent_id": subagent_id}),
                            is_error: false,
                        });
                    } else {
                        // Subagent delegation: collect the future, run it in parallel later.
                        // 阶段4 第1项（子目标 3/5）：继承上下文一次打包（delegate_ctx）。
                        let args_val_clone = args_val.clone();
                        let ctx = super::delegate_ctx::DelegateContext {
                            window: window.clone(),
                            app: app_c.clone(),
                            client: client.clone(),
                            api_url: api_url.clone(),
                            api_key: api_key.clone(),
                            model: model.clone(),
                            parent_assistant_id: assistant_id_c.clone(),
                            parent_topic_id: topic_id_c.clone(),
                            project_id: project_id_c.clone(),
                            agent_mode: agent_mode.clone(),
                            token: token_inner.clone(),
                            profile_model_overrides: profile_overrides_c.clone(),
                            custom_profiles: custom_profiles_c.clone(),
                            mcp_server_ids: mcp_server_ids_c.clone(),
                            semaphore: subagent_semaphore.clone(),
                            locale: locale_c.clone(),
                        };

                        let fut =
                            Box::pin(
                                async move { handle_delegate_task(&ctx, &args_val_clone).await },
                            );
                        parallel_futures.push((i, fut));
                    }
                } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                    && tc.name == tool_names::DELEGATE_TASKS
                {
                    // Batch parallel: collect one future into parallel_futures.
                    // 阶段4 第1项（子目标 3/5）：继承上下文一次打包（delegate_ctx）。
                    let args_val_clone = args_val.clone();
                    let ctx = super::delegate_ctx::DelegateContext {
                        window: window.clone(),
                        app: app_c.clone(),
                        client: client.clone(),
                        api_url: api_url.clone(),
                        api_key: api_key.clone(),
                        model: model.clone(),
                        parent_assistant_id: assistant_id_c.clone(),
                        parent_topic_id: topic_id_c.clone(),
                        project_id: project_id_c.clone(),
                        agent_mode: agent_mode.clone(),
                        token: token_inner.clone(),
                        profile_model_overrides: profile_overrides_c.clone(),
                        custom_profiles: custom_profiles_c.clone(),
                        mcp_server_ids: mcp_server_ids_c.clone(),
                        semaphore: subagent_semaphore.clone(),
                        locale: locale_c.clone(),
                    };

                    let fut =
                        Box::pin(async move { handle_delegate_tasks(&ctx, &args_val_clone).await });
                    parallel_futures.push((i, fut));
                } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                    && tc.name == tool_names::GET_SUBAGENT_RESULT
                {
                    // Pull 通道：查询/等待后台子智能体结果。与 delegate 系一致进阶段 2
                    // 并行池（同轮可并行等待多个后台任务；自身无副作用）。
                    let app_clone = app_c.clone();
                    let args_val_clone = args_val.clone();
                    let token_clone = token_inner.clone();
                    let fut = Box::pin(async move {
                        execute_get_subagent_result(&app_clone, &args_val_clone, &token_clone).await
                    });
                    parallel_futures.push((i, fut));
                } else {
                    // Non-delegate tool: execute immediately in order
                    // Extract the file path before tool execution (args_val may be moved into the MCP call)
                    let file_path_for_diff: Option<String> = if [
                        tool_names::WRITE_FILE,
                        tool_names::REPLACE_IN_FILE,
                        tool_names::DELETE_FILE,
                    ]
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
                        && matches!(
                            tc.name.as_str(),
                            tool_names::CREATE_TASKS | tool_names::UPDATE_TASK | "list_tasks"
                        )
                    {
                        // Orchestrator task board: in-memory metadata + frontend mirror,
                        // dispatched before the permission path (read_artifact precedent)
                        Ok(execute_task_board_tool(
                            &app_c,
                            &assistant_id_c,
                            &topic_id_c,
                            &tc.name,
                            &args_val,
                        ))
                    } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                        && tc.name == tool_names::ASK_USER
                    {
                        // Clarification questions: pauses the loop until the user answers
                        // (respond_agent_question), the token cancels, or the timeout fires
                        execute_ask_user(
                            &app_c,
                            &assistant_id_c,
                            &topic_id_c,
                            &args_val,
                            &token_inner,
                        )
                        .await
                    } else if server_id.as_deref() == Some(crate::core::ids::BUILTIN_SERVER_ID)
                        && app_config.parallel_tool_execution
                        && is_parallel_safe(&tc.name)
                    {
                        // Read-only built-in tool: execute concurrently (semantics follow Claude Code /
                        // Codex CLI parallel tool calling). Collected as a future that shares the phase-2
                        // FuturesUnordered with subagent delegations, emitting on completion; mutating tools
                        // are unaffected (still serial in original order), role:tool backfill keeps index order.
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
                                "main-agent",
                                None,
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
                            "main-agent",
                            None,
                        )
                        .await
                    } else {
                        // MCP tools go through the auto-retry wrapper (network errors are retried, permission/argument errors are not)
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
                                        None => Err(ierr!(
                                            "工具 {} 的 MCP server 未连接或意外断开，请检查 MCP 服务器状态",
                                            "The MCP server for tool {} is not connected or dropped unexpectedly; check the MCP server status",
                                            tn
                                        )),
                                    }
                                }
                            },
                            &ac,
                            &tok,
                        ).await
                    };

                    // Shared display-triple conversion (round_artifacts)
                    let (mut content_text, result_value, is_error) =
                        super::round_artifacts::tool_result_display(&tool_result);
                    // Circuit breaker v2 (harness round 3): only ENV-class failures
                    // count toward the disable; from the warning band an early-
                    // warning line rides along with the result text.
                    let outcome = classify_outcome(is_error, &content_text);
                    if let Some(n) = tool_breaker.record(&tc.name, outcome) {
                        content_text.push_str(&tool_breaker.warn_message(&tc.name, n));
                    }
                    // For file-modifying tools, capture the diff for frontend display
                    let file_changes = file_path_for_diff
                        .and_then(|path| {
                            file_tools::resolve_project_root(&app_c, project_id_c.as_deref())
                                .ok()
                                .and_then(|root| git_tools::capture_file_diff(&root, &path))
                        })
                        .map(|fc| vec![fc]);
                    // Emit the result immediately (without waiting for other tools)
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

                    // Backfill the role:tool message immediately (shared construction)
                    super::round_artifacts::push_tool_message(
                        &mut messages_for_api,
                        &tc.id,
                        &tc.name,
                        &truncate_tool_result(&content_text),
                    );
                }
            }

            // Phase 2 - run subagent delegations and read-only built-in tools concurrently (FuturesUnordered)
            if !parallel_futures.is_empty() {
                let mut unordered = FuturesUnordered::new();
                for (i, fut) in parallel_futures {
                    unordered.push(async move {
                        let result = fut.await;
                        (i, result)
                    });
                }

                // LOOP-5: the wait itself must honor cancellation — a queued
                // delegation (or any slow parallel tool) must not pin the turn open
                // for minutes after the user pressed stop. On cancel the remaining
                // futures are dropped; their inner tasks share the token and wind
                // down, and the round's bookkeeping below still runs.
                loop {
                    let (i, result) = tokio::select! {
                        _ = token_inner.cancelled() => {
                            was_cancelled = true;
                            break;
                        }
                        next = unordered.next() => match next {
                            Some(pair) => pair,
                            None => break,
                        },
                    };
                    // Shared display-triple conversion (round_artifacts)
                    let (mut content_text, result_value, is_error) =
                        super::round_artifacts::tool_result_display(&result);
                    // Circuit breaker v2 (harness round 3): env-class counting + warning band
                    let outcome = classify_outcome(is_error, &content_text);
                    if let Some(n) = tool_breaker.record(&round_result.tool_calls[i].name, outcome)
                    {
                        content_text.push_str(
                            &tool_breaker.warn_message(&round_result.tool_calls[i].name, n),
                        );
                    }
                    // Emit this delegate result immediately
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

                    // Backfill the role:tool message immediately (shared construction)
                    super::round_artifacts::push_tool_message(
                        &mut messages_for_api,
                        &tc.id,
                        &tc.name,
                        &truncate_tool_result(&content_text),
                    );
                }
            }

            // Emit results in original index order + backfill role:tool messages
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

                    super::round_artifacts::push_tool_message(
                        &mut messages_for_api,
                        &tc.id,
                        &tc.name,
                        &truncate_tool_result(&result.content_text),
                    );
                }
            }

            // Context budget check (token-level, model-aware): estimate the context, compress past the threshold
            // LOOP-8: the estimate (near-budget: exact tiktoken over the whole context) runs on the blocking pool.
            let estimated_tokens: usize =
                estimate_context_tokens_offload(&model, &messages_for_api, context_budget).await;
            if estimated_tokens > context_budget && messages_for_api.len() > 4 {
                // 阶段4 第1项（agent_loop 切片4）：压缩调用经共享 CompressCtx（事件仍在此发射——主循环专属）
                match compress_ctx.compress(&mut messages_for_api).await {
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
            }

            // Checkpoint: persist a snapshot at the end of EACH round (LOOP-2: this
            // block used to sit after the loop — a crash mid-turn then had NO active
            // checkpoint and resume always restarted the turn from scratch, despite
            // the module doc and retention policy promising per-round snapshots).
            // The turn's context is settled here: tool backfill + compression done.
            // LOOP-7: serialize BEFORE taking the global SQLite lock — serializing
            // a multi-MB context under `DbState` blocked every other database user.
            {
                let messages_json = serde_json::to_string(&messages_for_api).unwrap_or_default();
                let conn = app_c.state::<crate::core::state::DbState>();
                crate::commands::llm::checkpoints::save_checkpoint_json(
                    &conn.0.lock(),
                    &topic_id_c,
                    &turn_id,
                    round,
                    &messages_json,
                );
            }
        }

        // ===== Incomplete task -> handoff document =====
        // Triggered by: forced stop (round limit), error, or a truncated final response (length)
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

        // Background automatic fact extraction (sleep-time compute) - does not block the main flow
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

        // ===== Epilogue (guaranteed): always emit terminal done on normal stop, cancel, or error =====
        // Reaching the epilogue = non-crash termination -> checkpoints get marked terminal;
        // only crash leftovers (status still active) are the "recoverable" signal.
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

        // Remove self from global state (LOOP-1: identity-matched remove — after a
        // replacement raced past the 10s wait, this entry may already belong to the
        // NEW task, and a keyed remove would orphan it)
        // 阶段5: protocol on StreamManager
        state_inner.remove_own(&task_key_inner, &turn_id_c);
    }
}
