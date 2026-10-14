pub mod btw;

use crate::core::events as ev;
use crate::core::models::*;
use crate::core::permission::{self, PermissionAction};
use crate::core::state::DbState;
use crate::core::state::StreamManager;
use crate::ierr;
use crate::plugins::mcp::PendingApprovals;
use crate::utils::diff::compute_diff_preview;
use crate::utils::file_tools;
use crate::utils::git_tools;
use crate::utils::lsp_agent_tools;
use crate::utils::lsp_tools;
use crate::utils::shell_tools;
use crate::utils::web_tools;
use serde_json::json;
use std::future::Future;
use tauri::{AppHandle, Emitter, Manager, Window}; // Emitter pushes backend->frontend events
use tokio_util::sync::CancellationToken;

pub(crate) mod budget;
pub(crate) mod checkpoints;
pub(crate) mod client;
pub(crate) mod compress;
pub(crate) mod delegate_ctx;
pub(crate) mod handoff;
pub(crate) mod locked_tools;
pub(crate) mod messages;
// 阶段4 第1项（子目标 5/5 首切片）：两循环共享的轮次产物构造（agent_loop 地基）
pub(crate) mod round_artifacts;
pub(crate) mod sandbox_expand;
// Shared compaction-summary directive (pi structure + DSH verbatim rules), used by
// both the frontend-triggered summarize_history and the in-run compress path
pub(crate) mod sse_lines;
pub(crate) mod streaming;
pub(crate) mod subagents;
pub(crate) mod summarize_prompt;
pub(crate) mod tasks;
pub(crate) mod titles;
pub(crate) mod tool_dispatch;
pub(crate) mod tool_specs;
pub(crate) mod turn_runner;
pub(crate) mod usage;
pub(crate) mod vision;

// 阶段4 第1项：14 个子模块的 glob re-export 已移除——lib.rs 经由
// `llm::<module>::<command>` 全路径注册（同 checkpoints/tasks 的既有风格），
// 消除“同一符号两条路径”带来的漂移面。子模块间引用本就走全路径。
// mod.rs 自身使用的子模块符号改为下方显式 use（符号 → 真实来源模块）。
use crate::core::tool_names;
use budget::is_retryable_error;
use checkpoints::load_checkpoint_messages;
use client::{non_streaming_http_client, streaming_http_client};
use delegate_ctx::DelegateContext;
use locked_tools::execute_locked_file_tool;
use messages::{finish_message_for_api, message_metadata_for_api, MessageApiJob};
use turn_runner::AgentTurnRunner;
use usage::insert_usage_log;

/// Round and context budgets for the agent loop (main loop derives from model window;
/// subagents use conservative constants but honor explicit user config for rounds).
///
/// Both main and subagent loops derive budgets here so behavior stays aligned; the
/// subagent default is 100 rounds / 45k tokens, overridable by `AppConfig.max_tool_rounds`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AgentLimits {
    /// Max model-call rounds per run (runaway guard; on hit, finish gracefully with a progress summary)
    pub max_rounds: u32,
    /// Context token budget: exceeding it triggers compression
    pub context_budget: usize,
    /// Token budget for the retained recent region after compression
    pub keep_recent_tokens: usize,
}

impl AgentLimits {
    /// Subagent budget: default 100-round runaway guard (no low round cap; context
    /// management is the backstop); explicit user max_tool_rounds takes precedence.
    /// Context safety valve is a conservative constant (no model window info needed; safe for >=64k windows).
    pub(crate) fn for_subagent(cfg: &AppConfig) -> Self {
        /// Runaway-guard round count (subagent default cap)
        const GUARD_ROUNDS: u32 = 100;
        /// Conservative context budget (tokens)
        const CONTEXT_BUDGET: usize = 45_000;
        Self {
            max_rounds: effective_max_rounds(cfg.max_tool_rounds, GUARD_ROUNDS),
            context_budget: CONTEXT_BUDGET,
            keep_recent_tokens: (CONTEXT_BUDGET / 2).clamp(4_000, 20_000),
        }
    }
}

/// Effective max tool-call rounds from the user config: `-1` (or any negative) =
/// unlimited (完全权限 mode is inherently full-autonomy — the context budget,
/// compression and the stop button remain the backstops); `0` is invalid (the UI
/// forbids it) and clamps to the minimum 1; `None` falls back to the caller's guard
/// default.
pub(crate) fn effective_max_rounds(v: Option<i32>, guard: u32) -> u32 {
    match v {
        Some(n) if n < 0 => u32::MAX,
        Some(0) => 1,
        Some(n) => (n as u32).max(1),
        None => guard,
    }
}

/// Defensive check: scans messages for assistant(tool_calls) vs role:tool pairing,
/// logging a warning on missing or mismatched entries to aid debugging.
fn verify_tool_messages(messages: &[serde_json::Value]) {
    // Scan forward, tracking tool_call_ids declared by each assistant message
    let mut pending_ids: Vec<String> = Vec::new();
    let mut assistant_idx: Option<usize> = None;

    for (i, msg) in messages.iter().enumerate() {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("");
        if role == "assistant" {
            if let Some(tcs) = msg.get("tool_calls").and_then(|v| v.as_array()) {
                if !tcs.is_empty() {
                    // New assistant(tool_calls) starts while earlier pending_ids are still unmatched -> missing responses
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

    // Scan finished with tool_call_ids still unmatched
    if !pending_ids.is_empty() {
        tracing::warn!(
            "[verify_tool_messages] 扫描结束: assistant[{}] 的 tool_calls {:?} 缺少对应 tool 响应",
            assistant_idx.unwrap_or(0),
            pending_ids
        );
    }
}

/// Core function: calls the LLM and streams the result back in chunks.
///
/// Note: this command is kept for command stability; the new frontend flow uses [`run_agent_turn`]
/// (the backend single-task autonomous loop). It now only does single-round streaming + a terminal
/// done, with no tool execution/recursion -- tool-call notification events are still emitted (for debugging/compatibility).
#[tauri::command]
#[allow(
    clippy::too_many_arguments,
    reason = "Tauri IPC compatibility requires the existing flat command parameters"
)]
pub async fn call_llm_stream(
    window: Window,                         // Tauri window handle, used to emit events
    state: tauri::State<'_, StreamManager>, // Global state, manages in-flight stream tasks
    db_state: tauri::State<'_, DbState>,
    api_url: String,              // API endpoint URL
    api_key: String,              // API key
    model: String,                // Model name (e.g. gpt-3.5-turbo)
    assistant_id: String,         // Assistant ID (frontend matches messages by it)
    topic_id: String,             // Topic/session ID
    messages: Vec<Message>,       // History context messages
    tools: Option<Vec<ToolSpec>>, // Tool definitions (MCP tools; None or empty array means not sent)
    // Reasoning effort for reasoning-capable models ("low"/"medium"/"high"); None/off = no reasoning parameter
    reasoning_level: Option<String>,
) -> Result<(), String> {
    // Diagnostic logging (debug only): prints each message's role and which key fields are present.
    // Must stay at debug level -- production defaults to warn; conversation-content logs must not reach production logs.
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
    // SSRF protection
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    // 1. Build the task key (structured tuple, avoids "-" concatenation ambiguity)
    let task_key = (assistant_id.clone(), topic_id.clone());

    // 2. If a task is already running for this key, cancel it first (cancel, not abort,
    //    so the epilogue still runs). LOOP-1: legacy path previously returned without
    //    waiting at all — the old epilogue's keyed remove could then delete the NEW
    //    task's entry. Now it waits like run_agent_turn, and the removal below is
    //    identity-matched (turn_id) so an outliving old task is harmless.
    // 阶段5: the replacement protocol lives on StreamManager (single source, tested).
    state
        .replace_generation(&task_key, std::time::Duration::from_secs(10))
        .await;

    // 3. Clone variables for the async task (move closure)
    let state_inner = state.inner().clone();
    let task_key_inner = task_key.clone();
    let assistant_id_c = assistant_id.clone();
    let topic_id_c = topic_id.clone();
    // LOOP-10: two-phase conversion — the lock spans ONLY the SQLite metadata
    // pass; image disk reads + base64 run after release (the old single pass
    // held the global DB lock across every image read of the whole history).
    let messages_for_api = {
        let conn = db_state.0.lock();
        messages
            .iter()
            .map(|message| message_metadata_for_api(&conn, message))
            .collect::<Result<Vec<_>, _>>()?
    };
    let messages_for_api = messages_for_api
        .into_iter()
        .map(finish_message_for_api)
        .collect::<Result<Vec<_>, _>>()?;
    let tools_slice = tools; // kept for as_slice()

    // Defensive validation (debug builds only): checks tool_calls match tool responses.
    // The per-message JSON scan exists only for logging; release builds skip the cost.
    #[cfg(debug_assertions)]
    verify_tool_messages(&messages_for_api);

    let token = CancellationToken::new();
    let token_inner = token.clone();
    let app_handle = window.app_handle().clone();
    let model_c = model.clone();
    // LOOP-1：本回合代次号（epilogue 身份匹配删除）
    let stream_turn_id = uuid::Uuid::new_v4().to_string();
    let stream_turn_id_inner = stream_turn_id.clone();

    // 4. Spawn the async task that performs the request
    let handle = tokio::spawn(async move {
        let client = streaming_http_client();
        let tools_ref: Option<&[ToolSpec]> = tools_slice.as_deref();
        let result = streaming::stream_one_round(
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
            None, // main-conversation call: deltas emit llm-chunk / llm-reasoning directly
            Some(&app_handle),
            true,
            None,
            crate::core::models::default_model_call_retries(),
            reasoning_level.as_deref(),
        )
        .await;

        // Epilogue: emit a terminal done on success/cancel/error alike so the frontend always resets isThinking
        match result {
            Ok(round) => {
                // Persist this round's token usage to usage_log
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

        // Remove the task from global state once it finishes (identity-matched, LOOP-1;
        // 阶段5: protocol on StreamManager)
        state_inner.remove_own(&task_key_inner, &stream_turn_id_inner);
    });

    // 5. Store the running task handle and cancellation token in global state
    state.register(task_key, handle, token, stream_turn_id);
    Ok(())
}

/// Helper: fetches the list of available models from the provider
#[tauri::command]
pub async fn fetch_models(api_url: String, api_key: String) -> Result<Vec<ModelInfo>, String> {
    // SSRF protection
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    // Build the model-list URL, usually the base URL plus /models
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

    // Parse the returned model JSON
    let res_data: ModelsResponse = response.json().await.map_err(|e| e.to_string())?;
    Ok(res_data.data)
}

/// Stop function: called when the user clicks "Stop generating".
///
/// Notifies the task to exit gracefully via `CancellationToken::cancel()` (not `abort()`),
/// so after the `select!` branch returns the task still runs its epilogue
/// (emit done + remove itself), guaranteeing the frontend `isThinking` state is reset.
#[tauri::command]
pub async fn stop_llm_stream(
    state: tauri::State<'_, StreamManager>,
    assistant_id: String,
    topic_id: String,
) -> Result<(), String> {
    let task_key = (assistant_id.clone(), topic_id.clone());

    // Take the cancellation token and cancel; the task itself emits done and removes itself from state.
    // Note: cancel only, never abort, so the epilogue is guaranteed.
    // 阶段5: the stop protocol lives on StreamManager (single source, tested).
    state.cancel(&task_key);
    Ok(())
}

/// Executes a built-in tool (direct in-process call, with permission checks and approval).
///
/// Covers file tools (file_tools) and command execution tools (shell_tools), bypassing the MCP channel.
/// The permission model matches `execute_tool_call`.
///
/// `writer_id` labels the calling agent for multi-agent write coordination ("main-agent"
/// or a subagent label): mutating single-path file tools take the per-file exclusive
/// lock (`core::file_locks::FileLockRegistry`) so same-file writes serialize across the
/// main agent and parallel subagents, and cross-writer edits must carry a fresh hashline
/// anchor (anchor_hash / expect_hash) to prevent silent lost updates.
///
/// `workspace_root` overrides the root the tool operates on (worktree-isolated subagents
/// pass their worktree path); permission rules and project memory stay bound to the
/// project root resolved from `project_id`.
/// 沙箱拒绝签名：access-denied（WinError 5 系）或转发解释器 not-found（沙箱内
/// PATH 目录不可读导致的「程序不存在」）。扩权触发与「授权后仍被拒」兜底判定共用。
#[cfg(windows)]
fn sandbox_denial_signature(result: &ToolResult, command: &str) -> bool {
    let text = round_artifacts::tool_result_text(result);
    let denied = shell_tools::is_sandbox_access_denied(&text);
    let not_found = shell_tools::forwarded_interpreter_of(command).is_some()
        && shell_tools::is_command_not_found_result(&text);
    denied || not_found
}

/// not-found 子签名（解释器/工具链不可见）。Auto 自愈中唯一允许升级为无沙箱
/// 重试的类别——与 access-denied 类（写目标被拒等，永不升级）区分开。
#[cfg(windows)]
fn sandbox_not_found_signature(result: &ToolResult, command: &str) -> bool {
    let text = round_artifacts::tool_result_text(result);
    shell_tools::forwarded_interpreter_of(command).is_some()
        && shell_tools::is_command_not_found_result(&text)
}

#[allow(
    clippy::too_many_arguments,
    reason = "explicit execution context incl. workspace isolation"
)]
pub(crate) async fn execute_builtin_tool(
    app: &AppHandle,
    tool_name: &str,
    arguments: &serde_json::Value,
    project_id: Option<&str>,
    agent_mode: &AgentMode,
    token: &CancellationToken,
    writer_id: &str,
    workspace_root: Option<&str>,
) -> Result<ToolResult, String> {
    let project_root = file_tools::resolve_project_root(app, project_id)?;
    // Tools operate on the workspace root when the caller runs isolated (git worktree);
    // permission rules + memory below stay on the main project root.
    let tools_root = workspace_root
        .map(|s| s.to_string())
        .unwrap_or_else(|| project_root.clone());

    // Fail-closed: when the sandbox is unavailable (no bwrap/sandbox-exec), the
    // fully-autonomous mode (完全权限) refuses command execution per config.
    // Normal/Plan/只读 modes still go through permission approval (Ask / Deny),
    // whose reason carries a sandbox-unavailable warning (see the Ask branch below).
    // Windows 修正（安全审查 P1）：sandboxWindows="off" 时 AppContainer 未启用——
    // sandbox_available() 在 Windows 恒为 true（Job Object 无外部依赖），若只查它，
    // Auto + off = 零确认零沙箱静默通过，sandboxRequired 形同虚设。现在一并校验
    // 沙箱模式本身。
    let sandbox_cfg = crate::core::config::load_app_config(app).unwrap_or_default();
    let windows_sandbox_disabled = cfg!(windows) && sandbox_cfg.sandbox_windows != "appcontainer";
    if tool_name == tool_names::EXECUTE_COMMAND
        && matches!(agent_mode, AgentMode::Auto)
        && (!crate::utils::sandbox::sandbox_available() || windows_sandbox_disabled)
        && sandbox_cfg.sandbox_required
    {
        return Err(ierr!(
            "沙箱不可用，已拒绝执行命令（fail-closed）。请安装 bubblewrap（Linux）或确认 sandbox-exec 存在（macOS）；Windows 下需将 sandboxWindows 设为 appcontainer；或显式将 sandboxRequired 设为 false 回退无沙箱执行（不推荐）。",
            "Sandbox unavailable; command execution refused (fail-closed). Install bubblewrap (Linux) or verify sandbox-exec exists (macOS); on Windows set sandboxWindows to appcontainer; or explicitly set sandboxRequired to false to fall back to unsandboxed execution (not recommended)."
        ));
    }

    // Permission check (matches execute_tool_call; server_id keeps crate::core::ids::BUILTIN_FS_SERVER_ID for compatibility with existing rules).
    // 只读 (Off) mode goes through the same rule engine: its builtin arm allows reads
    // and denies everything else (fail-closed), so no structural bypass here.
    let custom_rules = permission::load_permissions(Some(&project_root)).rules;
    let action = permission::check_permission(
        tool_name,
        crate::core::ids::BUILTIN_FS_SERVER_ID,
        arguments,
        agent_mode,
        &custom_rules,
    );

    // SEC-01: network=true was previously auto-allowed in Auto (permission Allow) —
    // one prompt injection could exfiltrate any user-readable file silently, so
    // network-declaring commands got a dedicated confirmation gate. Since 完全权限
    // (Auto) became the inherent full-autonomy tier (risk confirmed on mode entry,
    // exfiltration trade-off stated there), the gate no longer applies in Auto:
    // zero interruptions is the mode's contract. Other tiers keep the gate via their
    // own Ask actions. Deny rules still win above.
    let network_auto = tool_name == tool_names::EXECUTE_COMMAND
        && !matches!(agent_mode, AgentMode::Auto)
        && arguments["network"].as_bool().unwrap_or(false)
        && action == PermissionAction::Allow;
    let action = if network_auto {
        PermissionAction::Ask
    } else {
        action
    };

    match action {
        PermissionAction::Deny => {
            return Err(ierr!(
                    "工具 '{}' 已在当前模式下被安全策略禁止执行（Deny）。\n如需执行，请切换到完全权限模式或在项目权限设置中添加 allow 规则。",
                    "Tool '{}' is blocked by the security policy in the current mode (Deny).\nTo allow it, switch to full-permission mode or add an allow rule in project permission settings.",
                    tool_name
                ));
        }
        PermissionAction::Ask => {
            let pending = app.state::<PendingApprovals>();
            let mut reason = ierr!(
                "工具 '{}' 需要您的确认才能执行",
                "Tool '{}' needs your confirmation to execute",
                tool_name
            );
            // Command execution: dangerous-command annotation
            if tool_name == tool_names::EXECUTE_COMMAND {
                if let Some(cmd) = arguments["command"].as_str() {
                    if let Some(danger) = shell_tools::check_dangerous_command(cmd) {
                        let prefix = match danger.category {
                            shell_tools::DangerCategory::Critical => "🚨 严重危险",
                            shell_tools::DangerCategory::High => "⚠️ 高风险",
                            shell_tools::DangerCategory::Medium => "⚡ 中等风险",
                        };
                        reason = ierr!(
                            "{}\n\n{}\n\n命令: {}\n\n确认执行？",
                            "{}\n\n{}\n\nCommand: {}\n\nConfirm execution?",
                            prefix,
                            danger.risk,
                            cmd
                        );
                    } else {
                        reason = ierr!(
                            "命令: {}\n\n工具 '{}' 需要您的确认才能执行",
                            "Command: {}\n\nTool '{}' needs your confirmation to execute",
                            cmd,
                            tool_name
                        );
                    }
                }
            }
            // Network declaration and sandbox availability annotations: the approval info presents the full risk surface
            if tool_name == tool_names::EXECUTE_COMMAND {
                if network_auto {
                    reason.insert_str(
                        0,
                        &ierr!(
                            "自动模式：声明网络访问的命令需确认。\n\n",
                            "Auto mode: network-declaring commands need confirmation.\n\n"
                        ),
                    );
                }
                if arguments["network"].as_bool().unwrap_or(false) {
                    reason.push_str(&ierr!(
                        "\n\n⚠ 此命令声明需要网络访问",
                        "\n\n⚠ This command declares network access"
                    ));
                }
                if !crate::utils::sandbox::sandbox_available() {
                    reason.push_str(&ierr!(
                            "\n\n⚠ 沙箱不可用：此命令将在无 OS 级沙箱的环境执行",
                            "\n\n⚠ Sandbox unavailable: this command will run without an OS-level sandbox"
                        ));
                }
            }
            // File-modifying tools: precompute a diff preview
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

    // Dispatch and execute
    if tool_name == tool_names::EXECUTE_COMMAND {
        let command = arguments["command"].as_str().unwrap_or("").to_string();
        let timeout = arguments["timeout"].as_u64();
        // Network egress control: declared explicitly by the model; default false = no network inside the sandbox
        let allow_network = arguments["network"].as_bool().unwrap_or(false);
        // Windows AppContainer: when enabled in config, run under the OS-level sandbox with
        // the configured toolchain dirs + session grants (added via expansion approvals).
        #[cfg(windows)]
        let ac_dirs: Option<Vec<String>> =
            sandbox_expand::appcontainer_dirs_with_session_grants(app);
        #[cfg(not(windows))]
        let ac_dirs: Option<Vec<String>> = None;
        #[cfg(windows)]
        let sandboxed = ac_dirs.is_some();
        // 沙箱配置一次性读取：.git 历史保护（Codex 式，仅主项目根——worktree 隔离
        // 工作区的 .git 是指回主仓库的指针文件，拒写会破坏 worktree 内的 shell git
        // 流程，故隔离工作区跳过）+ 完全权限自愈的「跳出沙箱兜底」开关
        let sandbox_cfg = crate::core::config::load_app_config(app).unwrap_or_default();
        let protect_git_ro = sandbox_cfg.sandbox_protect_git && workspace_root.is_none();
        #[cfg(windows)]
        let unsandboxed_fallback = sandbox_cfg.sandbox_unsandboxed_fallback;
        let root = tools_root.clone();
        let command_for_run = command.clone();
        let run_command = |ac_dirs: Option<Vec<String>>, command: String| {
            let root = root.clone();
            async move {
                tokio::task::spawn_blocking(move || {
                    shell_tools::execute_command(
                        &command,
                        &root,
                        timeout,
                        allow_network,
                        ac_dirs,
                        protect_git_ro,
                    )
                })
                .await
                .unwrap_or_else(|_| shell_tools::tool_err("命令执行线程 panic"))
            }
        };
        let started = std::time::SystemTime::now();
        let result = run_command(ac_dirs, command_for_run.clone()).await;
        // Harness round 3 (sandbox expansion): an access-denied signature from inside
        // the AppContainer — or a not-found signature when the command targets a
        // forwarded interpreter (cmd /c python … → 9009 inside the sandbox) — becomes
        // a one-click remediation dialog (grant dir / unsandboxed once / ignore)
        // instead of a raw error the model blind-retries against. Ask at most once
        // per operation token per 30-min window; timeout degrades to the original
        // error (pre-expansion behavior, safe for unattended runs).
        // 完全权限（Auto）无人值守：不弹窗、不受 ask-once 窗口节流（该窗口只为
        // 弹窗防骚扰而设，节流 Auto 的自动扩权会让后续同操作命令在 30 分钟内
        // 裸吃拒绝），命中签名即自动补救。
        // 安全审查收窄（PR #151）：Auto 的补救面 = 「解释器/工具链不可见」类
        // （not-found + 有派生目录）会话授权重试；**access-denied 类（写目标被拒、
        // .git 保护、项目外路径）永不升级为无沙箱执行**——拒绝文本来自模型可影响的
        // 命令输出（伪造 "Access is denied" 即可骗过签名），不能作为突破边界的
        // 决策点。唯一的无沙箱重试：not-found + 授权未落地（EDR 拦 ACL 写）+
        // `sandboxUnsandboxedFallback` 显式开启（默认 false），且有每回合预算上限。
        #[cfg(windows)]
        let auto_expand = matches!(agent_mode, AgentMode::Auto);
        #[cfg(windows)]
        let mut result = if sandboxed
            && result.is_error
            && (auto_expand || sandbox_expand::should_ask(&command))
            && sandbox_denial_signature(&result, &command)
        {
            tracing::warn!(
                "[sandbox-expand] 触发扩权请求：命令「{}」被沙箱拒绝（首次该操作，30 分钟窗口）",
                command
            );
            sandbox_expand::mark_asked(&command);
            let err_text = round_artifacts::tool_result_text(&result);
            let hint_dir = shell_tools::forward_script_interpreter(&command)
                .1
                .map(|p| p.to_string_lossy().into_owned());
            // 完全权限 (Auto) is inherently full-autonomy: skip the dialog and apply
            // the recommended decision directly — session grant for the derived
            // directory when one is derivable. Custom Deny rules are unaffected
            // (checked before this point). Other tiers keep the remediation dialog.
            let decision = if auto_expand {
                tracing::warn!(
                    "[sandbox-expand] 自动扩权（完全权限模式）：命令「{}」→ 会话授权 {} 后重试",
                    command,
                    hint_dir.as_deref().unwrap_or("(无派生目录，不升级)")
                );
                Ok(sandbox_expand::auto_remediation(hint_dir.as_deref()))
            } else {
                sandbox_expand::request_sandbox_expansion(app, &command, &err_text, hint_dir, token)
                    .await
            };
            match decision {
                Ok(sandbox_expand::ExpansionDecision::GrantSessionRetry { dir }) => {
                    crate::utils::sandbox::add_session_rx_grant(&dir);
                    let retried = run_command(
                        sandbox_expand::appcontainer_dirs_with_session_grants(app),
                        command_for_run.clone(),
                    )
                    .await;
                    // 授权可能写不进去（EDR 拦 ACL 写，round 10/11 实录：对
                    // D:\Git\cmd 的 SetNamedSecurityInfoW 被 error 5 拒绝）。授权后
                    // 仍为 not-found 类 = 目录依旧不可见 —— 唯一允许无沙箱重试的
                    // 类别（解释器/工具链不可见），且需 `sandboxUnsandboxedFallback`
                    // 显式开启 + 每回合预算；access-denied 类永不升级。
                    if auto_expand
                        && retried.is_error
                        && sandbox_not_found_signature(&retried, &command_for_run)
                        && sandbox_expand::escalate_after_failed_grant(
                            true,
                            unsandboxed_fallback,
                        )
                    {
                        let mut escaped = run_command(None, command_for_run).await;
                        sandbox_expand::annotate_result(
                            &mut escaped,
                            &ierr!(
                                "[沙箱边界] 本次命令已脱离 AppContainer 沙箱执行（会话授权未落地 + sandboxUnsandboxedFallback 开启；network 声明在此路径下不生效）",
                                "[sandbox boundary] this command ran OUTSIDE the AppContainer sandbox (session grant failed to land + sandboxUnsandboxedFallback on; the network declaration is NOT enforced on this path)"
                            ),
                        );
                        tracing::warn!(
                            "[sandbox-expand] 授权未落地，已脱离沙箱重试：命令「{command}」"
                        );
                        escaped
                    } else {
                        // 授权落地且命令成功 → 目录永久记入工具链配置（后续会话
                        // 免重新授权；等价弹窗「永久记住」，Auto 风险确认已覆盖）
                        if auto_expand && !retried.is_error {
                            sandbox_expand::persist_toolchain_dir(app, &dir);
                        }
                        retried
                    }
                }
                Ok(sandbox_expand::ExpansionDecision::GrantPermanentRetry { dir }) => {
                    crate::utils::sandbox::add_session_rx_grant(&dir);
                    sandbox_expand::persist_toolchain_dir(app, &dir);
                    run_command(
                        sandbox_expand::appcontainer_dirs_with_session_grants(app),
                        command_for_run.clone(),
                    )
                    .await
                }
                Ok(sandbox_expand::ExpansionDecision::RunUnsandboxedOnce) => {
                    // 仅非 Auto 弹窗路径可达（用户显式选择「跳出沙箱执行」）
                    let mut escaped = run_command(None, command_for_run.clone()).await;
                    sandbox_expand::annotate_result(
                        &mut escaped,
                        &ierr!(
                            "[沙箱边界] 本次命令按用户选择脱离 AppContainer 沙箱执行（仅此一次）",
                            "[sandbox boundary] this command ran OUTSIDE the AppContainer sandbox (explicit user choice, once)"
                        ),
                    );
                    escaped
                }
                Ok(sandbox_expand::ExpansionDecision::Ignore) => result,
                Err(e) if e == "cancelled" => return Err("cancelled".into()),
                Err(e) => {
                    // timeout / channel closed → keep the original error (unattended
                    // degradation, by design) — but LEAVE THE EVIDENCE: round 9
                    // reported the dialog as "never triggered" with no trace of
                    // whether it fired-and-timed-out; this log settles such disputes.
                    tracing::warn!(
                        "[sandbox-expand] 扩权请求未获响应（{e}），按忽略处理返回原错误"
                    );
                    result
                }
            }
        } else {
            result
        };
        // `.git` 历史保护拒绝的引导文案：git 自身打印 "Permission denied"，不匹配
        // 扩权签名 → 模型会拿到一个无解释的权限错误。命中「.git + 拒绝」时补一句
        // 可执行的指引（字符串级启发，仅指导不构成安全决策）。
        #[cfg(windows)]
        if result.is_error
            && command.contains(".git")
            && shell_tools::is_sandbox_access_denied(&round_artifacts::tool_result_text(&result))
        {
            sandbox_expand::annotate_result(
                &mut result,
                &ierr!(
                    "[提示] 对 .git 的写操作受仓库历史保护（sandboxProtectGit）：请改用内置 git_* 工具（宿主进程执行），或经用户将 sandboxProtectGit 设为 false",
                    "[hint] writes to .git are blocked by repository-history protection (sandboxProtectGit): use the builtin git_* tools (host process), or ask the user to set sandboxProtectGit=false"
                ),
            );
        }
        // SEC-09: a successful command may have modified project files outside the
        // hashline protocol. Mark files changed during its window so the next agent
        // write to them is forced through the cross-writer gate (re-read first) —
        // without this, a formatter/build/codegen command silently invalidates the
        // model's stale file context and the next write clobbers its output.
        if !result.is_error {
            let registry: std::sync::Arc<crate::core::file_locks::FileLockRegistry> = app
                .state::<std::sync::Arc<crate::core::file_locks::FileLockRegistry>>()
                .inner()
                .clone();
            let scan_root = tools_root.clone();
            let marked = tokio::task::spawn_blocking(move || {
                registry.mark_command_touched_files(std::path::Path::new(&scan_root), started)
            })
            .await
            .unwrap_or(0);
            if marked > 0 {
                tracing::debug!(
                    "[sec-09] 命令窗口内 {} 个项目文件被触碰，已标记需重读",
                    marked
                );
            }
        }
        Ok(result)
    } else if tool_name == tool_names::WEB_FETCH {
        let url = arguments["url"].as_str().unwrap_or("");
        let max_bytes = arguments["max_bytes"].as_u64();
        Ok(web_tools::execute_web_fetch(url, max_bytes).await)
    } else if tool_name == tool_names::WEB_SEARCH {
        let query = arguments["query"].as_str().unwrap_or("");
        let count = arguments["count"].as_u64();
        Ok(web_tools::execute_web_search(query, count).await)
    } else if tool_name == "read_lints" {
        lsp_tools::execute(app, &tools_root, arguments).await
    } else if tool_name == "lsp_definition" {
        lsp_agent_tools::execute_lsp_definition(app, &tools_root, arguments).await
    } else if tool_name == "lsp_references" {
        lsp_agent_tools::execute_lsp_references(app, &tools_root, arguments).await
    } else if tool_name == "lsp_hover" {
        lsp_agent_tools::execute_lsp_hover(app, &tools_root, arguments).await
    } else if tool_name == "lsp_symbols" {
        lsp_agent_tools::execute_lsp_symbols(app, &tools_root, arguments).await
    } else if crate::services::memory::tools::MEMORY_TOOL_NAMES.contains(&tool_name) {
        // The memory tool name list is maintained centrally in services/memory/tools.rs
        // (including search_code); never hardcode a literal list here to avoid dispatch/spec drift.
        // Memory stays bound to the main project root even under worktree isolation (the
        // memory DB is per-project, not per-worktree).
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
    } else if tool_name == tool_names::THINK {
        Ok(crate::utils::think::execute(arguments))
    } else if tool_name == tool_names::PROJECT_MAP {
        // SEC-20: the tree walk is synchronous FS traversal — run it on the
        // blocking pool instead of stalling the async worker thread.
        let root_c = tools_root.clone();
        let res = tokio::task::spawn_blocking(move || crate::utils::project_map::execute(&root_c))
            .await
            .unwrap_or_else(|_| crate::utils::tool_result::tool_err("project_map 执行线程 panic"));
        Ok(res)
    } else if tool_name.starts_with("git_") {
        let tool_name_c = tool_name.to_string();
        let arguments_c = arguments.clone();
        let root_c = tools_root.clone();
        Ok(tokio::task::spawn_blocking(move || {
            git_tools::execute_git_tool(&tool_name_c, &arguments_c, &root_c)
        })
        .await
        .unwrap_or_else(|_| git_tools::tool_err("Git 工具执行线程 panic")))
    } else if matches!(
        tool_name,
        tool_names::READ_FILE
            | tool_names::WRITE_FILE
            | tool_names::REPLACE_IN_FILE
            | "search_files"
            | tool_names::SEARCH_CONTENT
            | tool_names::LIST_DIRECTORY
            | tool_names::DELETE_FILE
            | tool_names::MAKE_DIRECTORY
    ) {
        // File tools run under the multi-agent write-coordination policy: mutating
        // single-path tools take the per-file exclusive lock and cross-writer edits must
        // carry a fresh hashline anchor (see commands/llm/locked_tools.rs).
        execute_locked_file_tool(app, tool_name, arguments, &tools_root, token, writer_id).await
    } else {
        // Unknown tools must error out explicitly. Never route unmatched tool names to a
        // file_tools fallback, which would give the model a semantically ambiguous result.
        Ok(file_tools::tool_err(&ierr!(
            "未知工具: {}",
            "Unknown tool: {}",
            tool_name
        )))
    }
}

/// Helper: check if a directory is a git repo
/// Private helper of the main agent context (subagents use the same resolution).
pub(crate) fn is_git_repo(dir: &str) -> bool {
    std::path::Path::new(dir).join(".git").exists()
}

/// Tool execution wrapper with automatic retry (used only for MCP network calls).
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

/// Run context for one autonomous turn of the main agent loop (consolidates the former 15 flat parameters).
///
/// IPC compatibility: the Tauri command keeps the flat parameters (old frontend calls unchanged);
/// the command body is a thin wrapper that builds a TurnContext and passes it to the implementation.
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
    /// MCP server ids opted in by the assistant (opt-in; empty = no MCP tools)
    pub mcp_server_ids: Vec<String>,
    /// Checkpoint of the turn to resume (continue after a crash); None = a normal new turn
    pub resume_turn_id: Option<String>,
    /// Reasoning effort for reasoning-capable models ("low"/"medium"/"high"); None = no reasoning parameter
    pub reasoning_level: Option<String>,
}

/// Autonomous agent-loop command (the main entry point of the new frontend flow).
///
/// Runs one tokio task through stream -> detect tools -> permission/approval -> execute MCP -> feed results back -> recurse,
/// until the model stops issuing tool calls or the round limit is hit. The frontend only listens to events and renders:
/// - `llm-round-start`: push an empty assistant placeholder message
/// - `llm-chunk`: append text / `done:true` marks the real end of the whole turn
/// - `llm-reasoning`: append chain-of-thought
/// - `llm-tool-call`: show a "running" bubble (notification only; this command performs the execution)
/// - `llm-tool-result`: update bubble status + append the role:tool message
/// - `tool-approval-requested`: approval request
///
/// # Parameters
/// - `messages`: initial message list (including this turn's user message); the frontend prebuilds system/history/user
/// - `mcp_server_ids`: MCP server ids opted in by the assistant. In Agent mode, tools of connected
///   servers are injected; conflicting tool names are renamed to mcp__<server>__<tool>; tools from
///   untrusted servers still require per-call approval in Auto mode (see mcp::execute_tool_call)
/// - `_agent_mode`: legacy IPC parameter, accepted for compatibility but IGNORED —
///   the mode is derived server-side from the assistant's persisted record (MCP-13)
/// - `reasoning_level`: reasoning effort for reasoning-capable models ("low"/"medium"/"high");
///   None/off = no reasoning parameter is attached to the request
/// - `project_id`: project id (used to resolve project-level permission rules)
/// - `locale`: optional UI language; falls back to `zh-CN` when missing on legacy calls
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
    _agent_mode: AgentMode,
    project_id: Option<String>,
    web_search_enabled: bool,
    profile_model_overrides: Vec<ProfileModelOverride>,
    custom_subagent_profiles: Vec<crate::core::models::CustomSubagentProfile>,
    locale: Option<String>,
    context_window: Option<u32>,
    skills: Option<Vec<SkillConfig>>,
    resume_turn_id: Option<String>,
    // Reasoning effort for reasoning-capable models ("low"/"medium"/"high"); None = no reasoning parameter
    reasoning_level: Option<String>,
) -> Result<(), String> {
    let ctx = TurnContext {
        api_url,
        api_key,
        model,
        assistant_id: assistant_id.clone(),
        topic_id,
        messages,
        // MCP-13：模式由服务端裁决——IPC 传入的 agent_mode 可伪造（伪造 auto
        // 即获得宽松权限面），助手记录里的持久化模式才是权威来源；记录缺失时
        // 回退 Off（fail-closed，Off 模式工具一律不可用）。
        agent_mode: {
            let conn = db_state.0.lock();
            crate::commands::config::load_assistant_agent_mode(&conn, &assistant_id)
                .unwrap_or(AgentMode::Off)
        },
        project_id,
        web_search_enabled,
        profile_model_overrides,
        custom_subagent_profiles,
        locale,
        context_window,
        skills,
        mcp_server_ids,
        resume_turn_id,
        reasoning_level,
    };
    run_agent_turn_inner(app, window, db_state, stream_mgr, ctx).await
}

/// Stop a given background subagent (fire-and-forget subagents started by delegate_task with wait=false).
///
/// # Parameters
/// - `subagent_id`: background subagent id (returned in the start confirmation result)
///
/// # Returns
/// true = cancel signal sent; false = not found (already finished or nonexistent; idempotent success).
#[tauri::command]
pub fn stop_subagent(app: AppHandle, subagent_id: String) -> Result<bool, String> {
    let handles = app.state::<crate::core::state::SubagentHandles>();
    // The Ref guard must be released within this statement (a tail-expression match keeps temporaries alive to the block end, triggering E0597)
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

/// Implementation of the main agent's one-turn autonomous loop (parameters packed into TurnContext).
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
        reasoning_level,
    } = ctx;
    // Diagnostic log (debug only): print each message's role and whether key fields are present.
    // Keep this at debug level: production defaults to warn, and conversation-content logs must not land in production logs.
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
    // SSRF protection
    crate::utils::url_validation::validate_http_url(
        &api_url,
        &crate::utils::url_validation::HttpUrlOptions::local_engine(),
    )?;
    // 每回合重置「脱离沙箱重试」预算（Auto 自愈的安全上限，见 sandbox_expand）
    #[cfg(windows)]
    sandbox_expand::reset_unsandboxed_budget();
    let task_key = (assistant_id.clone(), topic_id.clone());

    // Cancel the old task for the same topic (cancel, not abort).
    // The old handle must be awaited to finish its epilogue; otherwise the old task's state removal
    // could run after the new insert and wrongly delete the new task's entry, leaving the new task orphaned.
    // (LOOP-1: the remove is now identity-matched, so even a task that outlives this 10s window —
    // e.g. stuck in a 300s command — can no longer corrupt the replacement's entry.)
    // 阶段5: the replacement protocol lives on StreamManager (single source, tested).
    stream_mgr
        .replace_generation(&task_key, std::time::Duration::from_secs(10))
        .await;

    // Convert messages to API format up front (incl. attachment image expansion).
    // LOOP-10: the lock covers ONLY the SQLite metadata pass — the image disk
    // reads + base64 used to run inside it (every turn re-read the whole history's
    // attachments while every other DB user waited).
    let message_jobs: Vec<MessageApiJob<'_>> = {
        let conn = db_state.0.lock();
        messages
            .iter()
            .map(|m| message_metadata_for_api(&conn, m))
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut messages_for_api: Vec<serde_json::Value> = message_jobs
        .into_iter()
        .map(finish_message_for_api)
        .collect::<Result<Vec<_>, _>>()?;
    // Checkpoint resume: replace with the full context snapshot of the last round before the crash (continue instead of restarting from scratch).
    // turn_id persists across this turn: resume reuses the original turn_id (snapshot continuation); a new turn generates a new id.
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
    // Defensive validation (debug builds only): release builds skip the logging cost of a full message scan
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

    // ===== Token efficiency: model-aware context budget =====
    // Compression trigger thresholds are derived from the model context window (passed by the frontend
    // from the model catalog, default 128k), following oh-my-pi's reserve policy.
    // LOOP-4 (see budget::context_budget + its regression test): the floor used to be
    // `.max(16_000)`, which pushed small-window local models (4k/8k GGUF are common)
    // to 16k — reserve (16384) then exceeded the window, budget hit 0, and every
    // single round re-compressed (one summary LLM call per round).
    let (context_window_n, _context_reserve, context_budget) =
        crate::commands::llm::budget::context_budget(context_window);
    let keep_recent_tokens = (context_budget / 2).clamp(2_000, 20_000); // recent-token budget kept when compressing

    // Reasoning models (OpenAI o1/o3/o4, gpt-5 series) reject the max_tokens field (they need max_completion_tokens),
    // so no output cap is injected for them to avoid API errors; other models get half the window as output budget.
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
    let turn_id_public = turn_id.clone();
    // LOOP-6：panic 兜底 epilogue 所需克隆（AgentTurnRunner 会 move 走原始变量）
    let window_g = window.clone();
    let app_c_g = app.clone();
    let assistant_id_g = assistant_id.clone();
    let topic_id_g = topic_id.clone();
    let task_key_g = task_key.clone();
    let turn_id_g = turn_id.clone();
    let state_inner_g = stream_mgr.inner().clone();

    // 阶段4 第1项（子目标 4/5）：主循环闭包 → AgentTurnRunner。
    // 历史：此处是 22 行 let-clone 捕获列表 + ~1230 行 async move 闭包——捕获集
    // 隐式、无结构。现在 24 个捕获变量打包为结构体（turn_runner.rs），body
    // 原样迁入 run() 并经解构重命名回到原局部名（循环体零改动）。
    let runner = AgentTurnRunner {
        window: window.clone(),
        app: app.clone(),
        token: token.clone(),
        streams: stream_mgr.inner().clone(),
        task_key: task_key.clone(),
        assistant_id: assistant_id.clone(),
        topic_id: topic_id.clone(),
        turn_gen_id: turn_id.clone(),
        turn_id: turn_id.clone(),
        project_id: project_id.clone(),
        profile_overrides: profile_model_overrides.clone(),
        custom_profiles: custom_subagent_profiles.clone(),
        mcp_server_ids: mcp_server_ids.clone(),
        locale: locale_c.clone(),
        agent_mode,
        skills,
        messages_for_api,
        api_url,
        api_key,
        model,
        web_search_enabled,
        keep_recent_tokens,
        max_output_tokens,
        context_budget,
        reasoning_level,
    };
    let body_fut = runner.run();

    let handle = tokio::spawn(async move {
        use futures_util::FutureExt as _;
        match std::panic::AssertUnwindSafe(body_fut).catch_unwind().await {
            Ok(()) => {}
            Err(panic_payload) => {
                // LOOP-6 兜底 epilogue（主体 panic 时到达）
                let msg = panic_payload
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic_payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".to_string());
                tracing::error!("[agent-turn] 主循环 panic（已执行兜底 epilogue）: {msg}");
                {
                    let conn = app_c_g.state::<crate::core::state::DbState>();
                    crate::commands::llm::checkpoints::finalize_checkpoints(
                        &conn.0.lock(),
                        &topic_id_g,
                        &turn_id_g,
                        "error",
                    );
                }
                let _ = window_g.emit(
                    ev::LLM_CHUNK,
                    StreamPayload {
                        assistant_id: assistant_id_g.clone(),
                        topic_id: topic_id_g.clone(),
                        content: format!("\n[Error: 内部错误（panic）: {msg}]"),
                        done: true,
                        error: Some(msg),
                        input_tokens: None,
                        output_tokens: None,
                        context_tokens: None,
                        cached_input_tokens: None,
                        images: None,
                    },
                );
                // 阶段5: protocol on StreamManager (identity-matched panic epilogue)
                state_inner_g.remove_own(&task_key_g, &turn_id_g);
            }
        }
    });

    stream_mgr.register(task_key, handle, token, turn_id_public);
    Ok(())
}
