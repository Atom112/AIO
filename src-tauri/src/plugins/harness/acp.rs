//! ACP (Agent Client Protocol) driver — the universal adapter for every
//! ACP-speaking CLI harness (`opencode acp` / `omp acp` / `gemini --acp` /
//! `dsh --profile acp` / the official claude & codex npm adapters, ...).
//!
//! Transport: JSON-RPC 2.0 over the child's stdio, one message per line — the
//! same framing the MCP and LSP clients already use, driven through
//! `utils/jsonrpc.rs`'s PendingMap. The `agent-client-protocol` crate supplies
//! the serde schema types for inbound parsing (tagged `SessionUpdate`
//! discrimination with tolerant field handling); outbound envelopes are
//! hand-built with `json!` so the wire shape stays fully under our control.
//!
//! Inbound tolerance: spec-evolution variants and unknown agent→client requests
//! never kill the session — unknown notifications are debug-logged, unknown
//! requests get a `-32601` reply so the agent cannot hang waiting on us.

use crate::ierr;
use crate::plugins::harness::acp_history::build_history;
use crate::plugins::harness::acp_subagents::{
    apply_patch_str, apply_subagent_update, content_text_of, ingest_subagent_events,
    legacy_subagent_meta, push_recent, subagent_info, SubagentRuntime,
};
use crate::plugins::harness::adapter::{
    resolve_acp_command, spawn_harness_child_env, DriverCtx, DriverHandle, DriverSpawn,
    HarnessAdapter, HarnessCommand,
};
use crate::plugins::harness::model::{
    HarnessApprovalOption, HarnessApprovalRequest, HarnessCommandInfo, HarnessConfigOption,
    HarnessConfigValue, HarnessEvent, HarnessFileChange, HarnessPlanEntry, HarnessSubagentInfo,
    HarnessUsage,
};
use crate::plugins::harness::registry::{SpawnOptionInject, SpawnOptionSpec};
use crate::utils::jsonrpc::{PendingMap, WaitError};
use agent_client_protocol::schema::v1::{
    RequestPermissionRequest, SessionNotification, SessionUpdate,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::{mpsc, oneshot};

/// Sentinel injected by `fail_all` when the transport dies: waiters treat it as
/// "agent process exited" instead of a real JSON-RPC response.
const TRANSPORT_CLOSED: &str = "__aio_transport_closed__";
/// Handshake budget for `initialize` — deliberately generous because `npx -y`
/// fetches the adapter package on first use.
const INITIALIZE_TIMEOUT_SECS: u64 = 180;
/// Handshake budget for session/new / session/resume.
const SESSION_SETUP_TIMEOUT_SECS: u64 = 60;

/// The universal ACP adapter.
pub struct AcpAdapter;

#[async_trait::async_trait]
impl HarnessAdapter for AcpAdapter {
    async fn start(
        &self,
        spawn: DriverSpawn,
        cmd_rx: mpsc::UnboundedReceiver<HarnessCommand>,
    ) -> Result<DriverHandle, String> {
        AcpDriver::launch(spawn, cmd_rx).await
    }
}

/// Shared state of one running ACP driver.
pub(super) struct AcpDriver {
    ctx: DriverCtx,
    tool_label: String,
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    pending: Arc<PendingMap<Value>>,
    session_id: Arc<parking_lot::Mutex<Option<String>>>,
    prompt_running: Arc<AtomicBool>,
    /// Registry-declared CLI-flag-backed options (e.g. omp --approval-mode):
    /// injected as spawn args, synthesized into the config-option set, and
    /// handled locally on SetConfigOption (never forwarded to the agent).
    spawn_option_specs: Vec<SpawnOptionSpec>,
    spawn_option_values: Arc<parking_lot::Mutex<HashMap<String, String>>>,
    /// Last agent-reported config-option set (merged with the synthesized
    /// spawn options on every emission).
    agent_config_options: Arc<parking_lot::Mutex<Vec<HarnessConfigOption>>>,
    /// While `session/load` is in flight, session updates are the historical
    /// replay: buffer them here instead of emitting deltas (the driver converts
    /// the buffer into one history snapshot when the response arrives).
    replay_buffer: Arc<parking_lot::Mutex<Option<Vec<HarnessEvent>>>>,
    /// ACP subagent RFD children (client capability `subagents`): child session
    /// id → current display state. Child-addressed updates are aggregated here
    /// instead of leaking into the parent transcript.
    subagents: Arc<parking_lot::Mutex<HashMap<String, SubagentRuntime>>>,
}

impl AcpDriver {
    /// Spawn the child, run the ACP handshake, then keep the driver alive on its
    /// command loop. Returns once the session is established (or failed).
    async fn launch(
        spawn: DriverSpawn,
        cmd_rx: mpsc::UnboundedReceiver<HarnessCommand>,
    ) -> Result<DriverHandle, String> {
        let tool_label = spawn.spec.name.to_string();
        let (program, mut args) =
            resolve_acp_command(&spawn.spec).map_err(|e| format!("[{tool_label}] {e}"))?;
        // Registry-declared spawn options (omp --approval-mode, goose
        // GOOSE_MODE, ...): inject the chosen values' flags/env at process start.
        let (opt_args, opt_env) =
            spawn_option_injections(&spawn.spec.spawn_options, &spawn.spawn_option_values);
        args.extend(opt_args);
        let mut extra_env = spawn.extra_env.clone();
        extra_env.extend(opt_env);
        let mut child =
            spawn_harness_child_env(&program, &args, &spawn.workspace_path, &extra_env)?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| format!("[{tool_label}] 子进程 stdout 不可用"))?;
        let stderr = child.stderr.take();
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("[{tool_label}] 子进程 stdin 不可用"))?;
        let child = Arc::new(tokio::sync::Mutex::new(child));

        let ctx = spawn.ctx;
        let pending: Arc<PendingMap<Value>> = Arc::new(PendingMap::new());
        let driver = Arc::new(AcpDriver {
            ctx: ctx.clone(),
            tool_label: tool_label.clone(),
            stdin: Arc::new(tokio::sync::Mutex::new(stdin)),
            pending: pending.clone(),
            session_id: Arc::new(parking_lot::Mutex::new(None)),
            prompt_running: Arc::new(AtomicBool::new(false)),
            spawn_option_specs: spawn.spec.spawn_options.clone(),
            spawn_option_values: Arc::new(parking_lot::Mutex::new(
                spawn.spawn_option_values.clone(),
            )),
            agent_config_options: Arc::new(parking_lot::Mutex::new(Vec::new())),
            replay_buffer: Arc::new(parking_lot::Mutex::new(None)),
            subagents: Arc::new(parking_lot::Mutex::new(HashMap::new())),
        });
        // Tools with declared spawn options show them immediately (the
        // agent-reported options merge in once session/new completes).
        if !driver.spawn_option_specs.is_empty() {
            ctx.emit_event(HarnessEvent::ConfigOptions {
                options: driver.merged_config_options(),
            });
        }

        // stderr → debug log (agents write startup/config noise here)
        if let Some(stderr) = stderr {
            let label = tool_label.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!("[harness:{label}] stderr: {line}");
                }
            });
        }

        // Read loop (must run before the handshake: responses arrive on stdout)
        {
            let d = driver.clone();
            tokio::spawn(async move {
                d.read_loop(stdout).await;
            });
        }

        // ===== Handshake =====
        ctx.emit_state("connecting", None);
        let version = ctx.client_version.clone();
        let (init_id, init_rx) = driver
            .send_request(
                "initialize",
                json!({
                    "protocolVersion": 1,
                    // `subagents` opts into the ACP subagent-session RFD:
                    // agents (Claude Code ACP, Codex ACP, ...) then expose each
                    // child as its own session id instead of flattening it into
                    // a Task tool call. Unknown capabilities are ignored.
                    "clientCapabilities": { "subagents": {} },
                    "clientInfo": { "name": "AIO", "title": "AIO", "version": version }
                }),
            )
            .await?;
        let init_resp = driver
            .wait_request(init_id, init_rx, INITIALIZE_TIMEOUT_SECS)
            .await
            .map_err(|e| format!("[{tool_label}] ACP 握手失败: {e}"))?;
        let agent_version = init_resp
            .pointer("/result/agentInfo/version")
            .and_then(Value::as_str)
            .unwrap_or("?");
        tracing::info!("[harness:{tool_label}] ACP 握手完成（agent {agent_version}）");
        // Advertised lifecycle capabilities. `loadSession` replays the agent's
        // whole transcript (the only way to recover history and context for an
        // agent that cannot resume); `sessionCapabilities.resume` reattaches
        // silently. Capability blocks are optional — when neither is advertised
        // we keep the legacy "try resume, fall back to new" behavior.
        let load_supported = init_resp
            .pointer("/result/agentCapabilities/loadSession")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let resume_supported = init_resp
            .pointer("/result/agentCapabilities/sessionCapabilities/resume")
            .map(|v| !v.is_null())
            .unwrap_or(false);

        // Reattach an existing session. Preference order:
        // - no AIO display history yet → session/load (replay becomes history);
        // - otherwise → session/resume (fast, silent; DB is the display copy);
        // - first choice fails → the other method (both restore agent context);
        // - both fail → fresh session/new (the old id is gone).
        let mut harness_sid: Option<String> = None;
        let mut session_resp: Option<Value> = None;
        let mut replayed: Vec<HarnessEvent> = Vec::new();
        if let Some(rid) = spawn.harness_session_id.clone() {
            // Child-addressed subagent updates are routed by comparing the
            // update's sessionId with this root; install it before replay.
            *driver.session_id.lock() = Some(rid.clone());
            let prefer_load = load_supported && (spawn.replay_history || !resume_supported);
            let mut failure = String::new();

            if prefer_load {
                match driver.load_session(&rid, &spawn.workspace_path).await {
                    Ok((resp, replay)) => {
                        harness_sid = Some(rid.clone());
                        session_resp = Some(resp);
                        replayed = replay;
                    }
                    Err(e) => failure = format!("load: {e}"),
                }
            }
            if harness_sid.is_none() {
                match driver.resume_session(&rid, &spawn.workspace_path).await {
                    Ok(resp) => {
                        harness_sid = Some(rid.clone());
                        session_resp = Some(resp);
                    }
                    Err(e) => {
                        failure = if failure.is_empty() {
                            format!("resume: {e}")
                        } else {
                            format!("{failure}; resume: {e}")
                        };
                    }
                }
            }
            if harness_sid.is_none() && !prefer_load && load_supported {
                match driver.load_session(&rid, &spawn.workspace_path).await {
                    Ok((resp, replay)) => {
                        harness_sid = Some(rid.clone());
                        session_resp = Some(resp);
                        replayed = replay;
                    }
                    Err(e) => failure = format!("{failure}; load: {e}"),
                }
            }
            if harness_sid.is_none() {
                tracing::info!(
                    "[harness:{tool_label}] 恢复会话 {rid} 失败（回退新会话）: {failure}"
                );
                ctx.emit_event(HarnessEvent::Notice {
                    message: ierr!(
                        "原工具会话无法恢复，已新建会话；AIO 中的历史记录仍保留。",
                        "The tool session could not be restored; a new session was started. AIO's recorded history is still kept."
                    ),
                });
            }
        }
        if harness_sid.is_none() {
            // The old root is gone; a fresh session id replaces it after the
            // response, and any stray child traffic must not be attributed to it.
            *driver.session_id.lock() = None;
            // mcpServers must be sent EXPLICITLY empty: DSH's validator marks the
            // field required and rejects its omission (verified against the live
            // agent during bring-up — an empty array is valid for every agent).
            let (new_id, new_rx) = driver
                .send_request(
                    "session/new",
                    json!({ "cwd": spawn.workspace_path, "mcpServers": [] }),
                )
                .await?;
            let resp = driver
                .wait_request(new_id, new_rx, SESSION_SETUP_TIMEOUT_SECS)
                .await
                .map_err(|e| format!("[{tool_label}] 建立会话失败: {e}"))?;
            harness_sid = resp
                .pointer("/result/sessionId")
                .or_else(|| resp.get("sessionId"))
                .and_then(Value::as_str)
                .map(str::to_string);
            session_resp = Some(resp);
        }

        if let Some(sid) = &harness_sid {
            *driver.session_id.lock() = Some(sid.clone());
            ctx.set_harness_session_id(sid);
        } else {
            // Agents that omit sessionId in the response: keep the driver alive —
            // prompts surface a clear error instead of guessing ids.
            tracing::warn!("[harness:{tool_label}] 会话响应缺少 sessionId");
        }
        ctx.emit_event(HarnessEvent::SessionInfo {
            harness_session_id: harness_sid,
            model: None,
        });
        // session/load replay → one history snapshot (already converted to the
        // chat Message model). Persist first so a webview crash after the event
        // still recovers the transcript.
        if !replayed.is_empty() {
            let messages = build_history(&replayed);
            if !messages.is_empty() {
                ctx.replace_history(&messages);
                ctx.emit_event(HarnessEvent::History { messages });
            }
        }
        // Config options (model / thought-level selectors): the session/new (or
        // resume) result carries the agent's full option set; store it and emit
        // the merged set (agent options + synthesized spawn options).
        if let Some(resp) = &session_resp {
            if let Some(options) = parse_acp_config_options(resp) {
                *driver.agent_config_options.lock() = options;
            }
        }
        if !driver.agent_config_options.lock().is_empty() || !driver.spawn_option_specs.is_empty() {
            ctx.emit_event(HarnessEvent::ConfigOptions {
                options: driver.merged_config_options(),
            });
        }
        ctx.emit_state("idle", None);

        // Command loop
        {
            let d = driver.clone();
            tokio::spawn(async move {
                d.command_loop(cmd_rx).await;
            });
        }

        Ok(DriverHandle { child: Some(child) })
    }

    /// Serialize + write one NDJSON line to the agent's stdin.
    async fn write_line(&self, line: &str) -> Result<(), String> {
        let mut w = self.stdin.lock().await;
        w.write_all(line.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        w.write_all(b"\n").await.map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Send a JSON-RPC request and register its pending slot.
    async fn send_request(
        &self,
        method: &str,
        params: Value,
    ) -> Result<(u64, oneshot::Receiver<Value>), String> {
        let (id, rx) = self.pending.register();
        let line = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        if let Err(e) = self.write_line(&line.to_string()).await {
            self.pending.cancel(id);
            return Err(e);
        }
        Ok((id, rx))
    }

    /// Await a pending request response (token-aware, timeout-bounded). A
    /// JSON-RPC error object in the response is surfaced as `Err(message)`.
    async fn wait_request(
        &self,
        id: u64,
        rx: oneshot::Receiver<Value>,
        timeout_secs: u64,
    ) -> Result<Value, String> {
        let waited = tokio::select! {
            _ = self.ctx.token.cancelled() => {
                self.pending.cancel(id);
                return Err("会话已停止".to_string());
            }
            r = self.pending.wait(
                id,
                rx,
                std::time::Duration::from_secs(timeout_secs),
            ) => match r {
                Ok(v) => v,
                Err(WaitError::Canceled) => return Err("连接已断开".to_string()),
                Err(WaitError::Timeout(_)) => {
                    return Err(format!("等待响应超时（{timeout_secs}s）"));
                }
            },
        };
        if waited.get(TRANSPORT_CLOSED).is_some() {
            return Err("agent 进程已退出".to_string());
        }
        if let Some(err) = waited.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("未知错误");
            // ACP agents put the validation detail into error.data (e.g. DSH's
            // zod-style field errors) — surface it so setup quirks are diagnosable.
            let detail = err
                .get("data")
                .filter(|d| !d.is_null())
                .map(|d| d.to_string())
                .unwrap_or_default();
            return Err(if detail.is_empty() {
                msg.to_string()
            } else {
                format!("{msg}: {detail}")
            });
        }
        Ok(waited)
    }

    /// Send one session-lifecycle request (`session/resume` / `session/load`).
    async fn session_lifecycle(
        &self,
        method: &str,
        session_id: &str,
        cwd: &str,
    ) -> Result<Value, String> {
        let (id, rx) = self
            .send_request(
                method,
                json!({ "sessionId": session_id, "cwd": cwd, "mcpServers": [] }),
            )
            .await?;
        self.wait_request(id, rx, SESSION_SETUP_TIMEOUT_SECS).await
    }

    /// Reattach without replay (fast path when AIO already has the display copy).
    async fn resume_session(&self, session_id: &str, cwd: &str) -> Result<Value, String> {
        self.session_lifecycle("session/resume", session_id, cwd)
            .await
    }

    /// Load an existing session and capture its replayed `session/update`s.
    /// The replay buffer is installed before the request so no notification can
    /// race past it; it is always cleared even on failure.
    async fn load_session(
        &self,
        session_id: &str,
        cwd: &str,
    ) -> Result<(Value, Vec<HarnessEvent>), String> {
        *self.replay_buffer.lock() = Some(Vec::new());
        let result = self
            .session_lifecycle("session/load", session_id, cwd)
            .await;
        let replay = self.replay_buffer.lock().take();
        match result {
            Ok(resp) => Ok((resp, replay.unwrap_or_default())),
            Err(e) => Err(e),
        }
    }

    /// The stdout reader: dispatches JSON-RPC messages until the transport dies.
    async fn read_loop(self: Arc<Self>, stdout: ChildStdout) {
        let mut lines = BufReader::new(stdout).lines();
        loop {
            let line = tokio::select! {
                _ = self.ctx.token.cancelled() => break,
                l = lines.next_line() => match l {
                    Ok(Some(l)) => l,
                    Ok(None) => break,
                    Err(e) => {
                        tracing::debug!("[harness:{}] stdout 读取结束: {e}", self.tool_label);
                        break;
                    }
                },
            };
            // Tolerate CRLF line endings: some ACP agents on Windows emit \r\n
            // (BufReader lines keep the trailing \r, which breaks JSON parsing).
            let line = line.trim_end_matches('\r');
            self.handle_line(line).await;
        }
        // Transport closed / cancelled: end the session for everyone — pending
        // waiters get the sentinel, approval waiters see the token cancel.
        self.ctx.token.cancel();
        self.pending.fail_all(json!({ TRANSPORT_CLOSED: true }));
    }

    /// Dispatch one raw NDJSON line.
    async fn handle_line(self: &Arc<Self>, line: &str) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            tracing::debug!("[harness:{}] 非 JSON 行: {line}", self.tool_label);
            return;
        };
        let method = v.get("method").and_then(Value::as_str);
        if let Some(method) = method {
            match method {
                "session/update" => self.handle_session_update(v.get("params")),
                "session/request_permission" => {
                    let d = self.clone();
                    tauri::async_runtime::spawn(async move {
                        d.handle_permission(v).await;
                    });
                }
                other => {
                    if v.get("id").is_some() {
                        // Unknown request: answer method-not-found so the agent
                        // cannot block forever awaiting a reply.
                        let reply = json!({
                            "jsonrpc": "2.0",
                            "id": v.get("id"),
                            "error": {
                                "code": -32601,
                                "message": format!("method not found: {other}")
                            }
                        });
                        let _ = self.write_line(&reply.to_string()).await;
                    } else {
                        tracing::debug!("[harness:{}] 忽略未知通知 {other}", self.tool_label);
                    }
                }
            }
        } else if let Some(id) = v.get("id").and_then(Value::as_u64) {
            // A response to one of our requests.
            if let Some(tx) = self.pending.take(id) {
                let _ = tx.send(v);
            }
        }
    }

    /// `session/update` → normalized events (typed parse, wire-pointer reads).
    /// Updates are addressed by `params.sessionId`: the root session maps to the
    /// normal GUI stream, any other id is a subagent child from the ACP
    /// subagent RFD and is aggregated into its card instead.
    fn handle_session_update(&self, params: Option<&Value>) {
        let Some(params) = params else { return };
        let outer = params
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or("");
        let root = self.session_id.lock().clone();
        let mut child =
            (!outer.is_empty() && root.as_deref() != Some(outer)).then(|| outer.to_string());
        // Some ACP agents (Qwen Code today) flatten subagent traffic into the
        // parent session and tag it with `_meta.parentToolCallId/subagentType`
        // instead of the RFD's child session ids.
        if child.is_none() {
            if let Some((parent_tool_call, agent)) = legacy_subagent_meta(params) {
                if let Some(agent) = agent {
                    let mut subagents = self.subagents.lock();
                    let runtime = subagents.entry(parent_tool_call.clone()).or_default();
                    runtime.agent = Some(agent.clone());
                    if runtime.title.is_none() {
                        runtime.title = Some(agent);
                    }
                }
                child = Some(parent_tool_call);
            }
        }

        if let Some(update) = params.get("update") {
            let kind = update
                .get("sessionUpdate")
                .and_then(Value::as_str)
                .unwrap_or("");
            if self.handle_rfd_update(child.as_deref(), kind, update) {
                return;
            }
            if child.is_some()
                && !matches!(
                    kind,
                    "agent_message_chunk"
                        | "agent_thought_chunk"
                        | "tool_call"
                        | "tool_call_update"
                        | "plan"
                        | "usage_update"
                        | "notice"
                        | "config_option_update"
                        | "current_mode_update"
                        | "session_info_update"
                        | "available_commands_update"
                )
            {
                tracing::debug!("[harness:{}] 忽略子会话更新 {kind}", self.tool_label);
                return;
            }
        } else {
            return;
        }

        match serde_json::from_value::<SessionNotification>(params.clone()) {
            Ok(note) => {
                let events = map_session_update(&self.tool_label, note.update);
                if let Some(child) = &child {
                    self.ingest_child_events(child, events);
                } else {
                    for event in events {
                        self.dispatch_root_event(event);
                    }
                }
            }
            Err(e) => {
                // Future spec variants must never kill the stream.
                tracing::debug!(
                    "[harness:{}] 无法解析 session/update（已跳过）: {e}",
                    self.tool_label
                );
            }
        }
    }

    /// Route one root-session event: config/command state is driver-local,
    /// everything else is buffered during history replay or emitted live.
    fn dispatch_root_event(&self, event: HarnessEvent) {
        match event {
            HarnessEvent::ConfigOptions { options } => {
                *self.agent_config_options.lock() = options;
                if self.replay_buffer.lock().is_none() {
                    self.ctx.emit_event(HarnessEvent::ConfigOptions {
                        options: self.merged_config_options(),
                    });
                }
            }
            // Command menus are useful immediately (the user can queue a slash
            // command while a long transcript replays); they are not history.
            HarnessEvent::CommandsUpdate { .. } => self.ctx.emit_event(event),
            other => self.emit_or_buffer(other),
        }
    }

    fn emit_or_buffer(&self, event: HarnessEvent) {
        let mut replay = self.replay_buffer.lock();
        if let Some(buffer) = replay.as_mut() {
            buffer.push(event);
        } else {
            drop(replay);
            self.ctx.emit_event(event);
        }
    }

    /// Aggregate one child-session event into its subagent card. Child text and
    /// tool calls never become parent transcript content.
    fn ingest_child_events(&self, child: &str, events: Vec<HarnessEvent>) {
        let changed = {
            let mut subagents = self.subagents.lock();
            let runtime = subagents.entry(child.to_string()).or_default();
            ingest_subagent_events(runtime, events)
        };
        if changed {
            self.emit_subagent_snapshot(child);
        }
    }

    /// Parse the subagent-session RFD notifications that the stable ACP schema
    /// does not type yet. Returns true when the update was handled here.
    fn handle_rfd_update(&self, child: Option<&str>, kind: &str, update: &Value) -> bool {
        match kind {
            "subagent_update" => {
                let Some(child_id) = update.get("sessionId").and_then(Value::as_str) else {
                    return true;
                };
                {
                    let mut subagents = self.subagents.lock();
                    let runtime = subagents.entry(child_id.to_string()).or_default();
                    apply_subagent_update(runtime, update);
                }
                self.emit_subagent_snapshot(child_id);
                true
            }
            "session_message" | "session_message_chunk" => {
                let text = content_text_of(update.get("content"));
                if let Some(text) = text {
                    let target = child.map(str::to_string).or_else(|| {
                        // The parent transcript's incoming view names the child
                        // as its sender; attribute it to that child's card.
                        update
                            .get("senderSessionId")
                            .and_then(Value::as_str)
                            .filter(|sid| self.subagents.lock().contains_key(*sid))
                            .map(str::to_string)
                    });
                    if let Some(target) = target {
                        {
                            let mut subagents = self.subagents.lock();
                            let runtime = subagents.entry(target.clone()).or_default();
                            push_recent(&mut runtime.recent_output, text, 2000);
                        }
                        self.emit_subagent_snapshot(&target);
                    }
                }
                true
            }
            // AIR's earlier draft (only sent when the client opts into AIR).
            "subagent_spawned" => {
                let Some(child_id) = update.get("subagentSessionId").and_then(Value::as_str) else {
                    return true;
                };
                {
                    let mut subagents = self.subagents.lock();
                    let runtime = subagents.entry(child_id.to_string()).or_default();
                    apply_patch_str(&mut runtime.title, update.get("name"));
                    apply_patch_str(&mut runtime.description, update.get("task"));
                    runtime.state = "running".to_string();
                }
                self.emit_subagent_snapshot(child_id);
                true
            }
            "subagent_state_update" => {
                let Some(child_id) = update.get("subagentSessionId").and_then(Value::as_str) else {
                    return true;
                };
                {
                    let mut subagents = self.subagents.lock();
                    let runtime = subagents.entry(child_id.to_string()).or_default();
                    runtime.state = "idle".to_string();
                    runtime.stop_reason = Some(
                        update
                            .get("state")
                            .and_then(Value::as_str)
                            .unwrap_or("completed")
                            .to_string(),
                    );
                }
                self.emit_subagent_snapshot(child_id);
                true
            }
            // Async-task opt-in is not advertised; tolerate the frames.
            "async_task_spawned" | "async_task_progress" | "async_task_state_update" => true,
            _ => false,
        }
    }

    /// Emit the current card state of one child (or buffer it during replay).
    fn emit_subagent_snapshot(&self, child: &str) {
        let info = {
            let subagents = self.subagents.lock();
            let Some(runtime) = subagents.get(child) else {
                return;
            };
            subagent_info(child, runtime)
        };
        self.dispatch_root_event(HarnessEvent::SubagentUpdate {
            entries: vec![info],
        });
    }

    /// `session/request_permission` → harness approval bubble → user decision →
    /// JSON-RPC result (`selected` + optionId, or `cancelled`).
    async fn handle_permission(&self, v: Value) {
        let Some(rid) = v.get("id").cloned() else {
            return;
        };
        let params = v.get("params").cloned().unwrap_or(Value::Null);
        let typed = match serde_json::from_value::<RequestPermissionRequest>(params) {
            Ok(t) => t,
            Err(e) => {
                let reply = json!({
                    "jsonrpc": "2.0", "id": rid,
                    "error": { "code": -32602, "message": format!("invalid params: {e}") }
                });
                let _ = self.write_line(&reply.to_string()).await;
                return;
            }
        };
        let wire = match serde_json::to_value(&typed) {
            Ok(w) => w,
            Err(_) => Value::Null,
        };
        let request = HarnessApprovalRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            kind: "tool".to_string(),
            title: wire
                .pointer("/toolCall/title")
                .and_then(Value::as_str)
                .map(str::to_string),
            tool_name: wire
                .pointer("/toolCall/name")
                .and_then(Value::as_str)
                .map(str::to_string),
            arguments: wire.pointer("/toolCall/rawInput").cloned(),
            command: None,
            file_path: preview_file_path(&wire),
            preview_diff: preview_diff_from_tool_call(&wire),
            options: wire
                .get("options")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|o| {
                            Some(HarnessApprovalOption {
                                option_id: o.get("optionId")?.as_str()?.to_string(),
                                name: o
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .unwrap_or("?")
                                    .to_string(),
                                kind: o
                                    .get("kind")
                                    .and_then(Value::as_str)
                                    .unwrap_or("allow_once")
                                    .to_string(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
        };
        let outcome = match self.ctx.request_approval(request).await {
            Some(decision) if !decision.cancelled => match decision.option_id {
                Some(option_id) => json!({
                    "outcome": { "outcome": "selected", "optionId": option_id }
                }),
                None => json!({ "outcome": { "outcome": "cancelled" } }),
            },
            _ => json!({ "outcome": { "outcome": "cancelled" } }),
        };
        let reply = json!({ "jsonrpc": "2.0", "id": rid, "result": outcome });
        let _ = self.write_line(&reply.to_string()).await;
    }

    /// Consume manager commands for the lifetime of the session.
    async fn command_loop(self: Arc<Self>, mut cmd_rx: mpsc::UnboundedReceiver<HarnessCommand>) {
        loop {
            let cmd = tokio::select! {
                _ = self.ctx.token.cancelled() => break,
                c = cmd_rx.recv() => match c {
                    Some(c) => c,
                    None => break,
                },
            };
            match cmd {
                HarnessCommand::Prompt { text } => self.handle_prompt(text).await,
                HarnessCommand::SetPermissionMode { .. } => {
                    // ACP v1 has no permission-mode surface (mode methods are
                    // deprecated config options); only native adapters act on it.
                    tracing::debug!(
                        "[harness:{}] 忽略 SetPermissionMode（ACP 无此界面）",
                        self.tool_label
                    );
                }
                HarnessCommand::SetConfigOption { config_id, value } => {
                    self.handle_set_config_option(config_id, value).await;
                }
                HarnessCommand::CancelTurn => {
                    let Some(sid) = self.session_id.lock().clone() else {
                        continue;
                    };
                    let note = json!({
                        "jsonrpc": "2.0",
                        "method": "session/cancel",
                        "params": { "sessionId": sid }
                    });
                    if let Err(e) = self.write_line(&note.to_string()).await {
                        tracing::debug!(
                            "[harness:{}] session/cancel 发送失败: {e}",
                            self.tool_label
                        );
                    }
                }
            }
        }
    }

    /// Submit a user prompt and spawn the turn-end waiter.
    async fn handle_prompt(self: &Arc<Self>, text: String) {
        if self
            .prompt_running
            .load(std::sync::atomic::Ordering::SeqCst)
        {
            self.ctx.emit_event(HarnessEvent::Error {
                message: "上一个回合仍在运行，请先停止或等待其结束".to_string(),
            });
            return;
        }
        let Some(sid) = self.session_id.lock().clone() else {
            self.ctx.emit_event(HarnessEvent::Error {
                message: "会话尚未建立（缺少 sessionId）".to_string(),
            });
            return;
        };
        let Ok((id, rx)) = self
            .send_request(
                "session/prompt",
                json!({
                    "sessionId": sid,
                    "prompt": [ { "type": "text", "text": text } ]
                }),
            )
            .await
        else {
            self.ctx.emit_event(HarnessEvent::Error {
                message: "发送失败：agent 进程已退出".to_string(),
            });
            return;
        };
        self.prompt_running
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.ctx.emit_event(HarnessEvent::TurnStarted);
        self.ctx.emit_state("running", None);
        let d = self.clone();
        tauri::async_runtime::spawn(async move {
            d.wait_turn(id, rx).await;
        });
    }

    /// Set a session config option. Registry-declared spawn options are
    /// handled locally (stored for the next spawn; never forwarded — the
    /// agent doesn't know them). Agent options go through
    /// `session/set_config_option` and the response's option set is stored +
    /// merged into the emitted view. Wire shapes verified from the ACP v1
    /// schema: select → `{sessionId, configId, value: "model-id"}`; boolean →
    /// `{sessionId, configId, type: "boolean", value: true}`.
    async fn handle_set_config_option(&self, config_id: String, value: Value) {
        // Spawn options: local state update + merged re-emit (the actual flag
        // applies at the next spawn; the GUI persists + stops the session).
        if self
            .spawn_option_specs
            .iter()
            .any(|s| s.option_id == config_id)
        {
            if let Some(v) = value.as_str() {
                self.spawn_option_values
                    .lock()
                    .insert(config_id, v.to_string());
                self.ctx.emit_event(HarnessEvent::ConfigOptions {
                    options: self.merged_config_options(),
                });
            }
            return;
        }
        let Some(sid) = self.session_id.lock().clone() else {
            return;
        };
        let params = if let Some(b) = value.as_bool() {
            json!({
                "sessionId": sid,
                "configId": config_id,
                "type": "boolean",
                "value": b
            })
        } else {
            json!({
                "sessionId": sid,
                "configId": config_id,
                "value": value
            })
        };
        let Ok((id, rx)) = self.send_request("session/set_config_option", params).await else {
            tracing::debug!("[harness:{}] set_config_option 发送失败", self.tool_label);
            return;
        };
        // Await the response: on success the agent's config_option_update
        // notification re-emits the new state; on error we surface it.
        if let Ok(resp) = self.wait_request(id, rx, SESSION_SETUP_TIMEOUT_SECS).await {
            if let Some(options) = parse_acp_config_options(&resp) {
                *self.agent_config_options.lock() = options;
                self.ctx.emit_event(HarnessEvent::ConfigOptions {
                    options: self.merged_config_options(),
                });
            }
        }
    }

    /// The config-option set for the GUI: agent-reported options followed by
    /// the registry-declared spawn options (synthesized with their current
    /// preference value, defaulting to the first value).
    fn merged_config_options(&self) -> Vec<HarnessConfigOption> {
        let mut options = self.agent_config_options.lock().clone();
        let values = self.spawn_option_values.lock();
        for spec in &self.spawn_option_specs {
            options.push(synth_spawn_option(spec, values.get(&spec.option_id)));
        }
        options
    }

    /// Turn-end waiter: resolves the pending session/prompt response into
    /// TurnEnded + idle state (handles cancel, JSON-RPC error, transport death).
    async fn wait_turn(&self, id: u64, rx: oneshot::Receiver<Value>) {
        let waited = tokio::select! {
            _ = self.ctx.token.cancelled() => {
                self.pending.cancel(id);
                self.prompt_running.store(false, std::sync::atomic::Ordering::SeqCst);
                // Hard stop: surface a cancelled turn so the frontend resets.
                self.ctx.emit_event(HarnessEvent::TurnEnded {
                    stop_reason: "cancelled".to_string(),
                    error: None,
                });
                self.ctx.emit_state("idle", None);
                return;
            }
            r = self.pending.wait(
                id,
                rx,
                std::time::Duration::from_secs(24 * 3600),
            ) => match r {
                Ok(v) => v,
                Err(_) => {
                    self.prompt_running.store(false, std::sync::atomic::Ordering::SeqCst);
                    return;
                }
            },
        };
        self.prompt_running
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let (stop_reason, error) = if waited.get(TRANSPORT_CLOSED).is_some() {
            ("error".to_string(), Some("agent 进程已退出".to_string()))
        } else if let Some(err) = waited.get("error") {
            (
                "error".to_string(),
                Some(
                    err.get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("未知错误")
                        .to_string(),
                ),
            )
        } else {
            let sr = waited
                .pointer("/result/stopReason")
                .or_else(|| waited.get("stopReason"))
                .and_then(Value::as_str)
                .unwrap_or("end_turn");
            (sr.to_string(), None)
        };
        self.ctx
            .emit_event(HarnessEvent::TurnEnded { stop_reason, error });
        self.ctx.emit_state("idle", None);
    }
}

// ===== SessionUpdate → HarnessEvent mapping =====
//
// Strategy: the crate deserializes the tagged `SessionUpdate` (tolerant,
// forward-compatible); each payload is then re-serialized to its wire JSON and
// read via pointers, so the mapping depends only on documented wire names —
// never on Rust field/type names that may shift between crate majors.

fn map_session_update(tool: &str, update: SessionUpdate) -> Vec<HarnessEvent> {
    match update {
        SessionUpdate::AgentMessageChunk(c) => {
            let wire = serde_json::to_value(&c).ok();
            text_of_chunk(wire.as_ref())
                .map(|t| vec![HarnessEvent::TextDelta { text: t }])
                .unwrap_or_default()
        }
        SessionUpdate::AgentThoughtChunk(c) => {
            let wire = serde_json::to_value(&c).ok();
            text_of_chunk(wire.as_ref())
                .map(|t| vec![HarnessEvent::ThinkingDelta { text: t }])
                .unwrap_or_default()
        }
        SessionUpdate::UserMessageChunk(c) => {
            // Live turns already render the user's own input; history replay
            // (session/load) uses this to reconstruct the transcript.
            let wire = serde_json::to_value(&c).ok();
            text_of_chunk(wire.as_ref())
                .map(|t| vec![HarnessEvent::UserTextDelta { text: t }])
                .unwrap_or_default()
        }
        SessionUpdate::ToolCall(tc) => serde_json::to_value(&tc)
            .ok()
            .map(|w| {
                let mut events = tool_call_events(&w);
                if let Some(subagents) = subagent_update_event(&w) {
                    events.push(subagents);
                }
                events
            })
            .unwrap_or_default(),
        SessionUpdate::ToolCallUpdate(u) => serde_json::to_value(&u)
            .ok()
            .map(|w| {
                let mut events = tool_call_update_events(&w);
                if let Some(subagents) = subagent_update_event(&w) {
                    events.push(subagents);
                }
                events
            })
            .unwrap_or_default(),
        SessionUpdate::Plan(p) => serde_json::to_value(&p)
            .ok()
            .and_then(|w| plan_event(&w))
            .map(|e| vec![e])
            .unwrap_or_default(),
        SessionUpdate::UsageUpdate(u) => serde_json::to_value(&u)
            .ok()
            .and_then(|w| usage_event(&w))
            .map(|e| vec![e])
            .unwrap_or_default(),
        SessionUpdate::Notice(n) => serde_json::to_value(&n)
            .ok()
            .and_then(|w| notice_event(&w))
            .map(|e| vec![e])
            .unwrap_or_default(),
        // Config option state changed (agent applied a set_config_option, or its
        // option set evolved): re-emit the full current set for the GUI.
        SessionUpdate::ConfigOptionUpdate(c) => {
            let wire = serde_json::to_value(&c).ok();
            wire.and_then(|w| {
                parse_acp_config_options(&w)
                    .map(|options| vec![HarnessEvent::ConfigOptions { options }])
            })
            .unwrap_or_default()
        }
        // Slash command menu (ACP has no invoke method — the GUI sends
        // `/name [input]` as the next prompt).
        SessionUpdate::AvailableCommandsUpdate(c) => {
            let wire = serde_json::to_value(&c).ok();
            wire.map(|w| {
                vec![HarnessEvent::CommandsUpdate {
                    commands: commands_of(&w),
                }]
            })
            .unwrap_or_default()
        }
        // Not consumed by the GUI (reserved for later PRs).
        SessionUpdate::CurrentModeUpdate(_)
        | SessionUpdate::SessionInfoUpdate(_)
        | SessionUpdate::CompactionUpdate(_)
        | SessionUpdate::CompactionSummaryChunk(_) => Vec::new(),
        // non_exhaustive: unknown future variants never break the stream.
        _ => {
            tracing::debug!("[harness:{tool}] 忽略未知 sessionUpdate 变体");
            Vec::new()
        }
    }
}

/// `{"messageId": ..., "content": {"type":"text","text": ...}}` → text.
fn text_of_chunk(wire: Option<&Value>) -> Option<String> {
    let t = wire?.pointer("/content/text")?.as_str()?.to_string();
    Some(t)
}

/// Full tool-call wire `{"toolCallId","title","name","kind","status","content",
/// "rawInput","rawOutput"}` → Start (pending/in_progress) or End (completed/failed).
fn tool_call_events(w: &Value) -> Vec<HarnessEvent> {
    let Some(tool_call_id) = w.get("toolCallId").and_then(Value::as_str) else {
        return Vec::new();
    };
    let status = w.get("status").and_then(Value::as_str).unwrap_or("pending");
    let (content, file_changes) = content_of_tool_call(w.get("content"));
    let output = content.or_else(|| raw_output_text(w));
    match status {
        "completed" => vec![HarnessEvent::ToolCallEnd {
            tool_call_id: tool_call_id.to_string(),
            ok: true,
            content: output,
            error: None,
            file_changes,
        }],
        "failed" => vec![HarnessEvent::ToolCallEnd {
            tool_call_id: tool_call_id.to_string(),
            ok: false,
            content: output.clone(),
            error: output.or(Some("tool failed".to_string())),
            file_changes,
        }],
        _ => vec![HarnessEvent::ToolCallStart {
            tool_call_id: tool_call_id.to_string(),
            name: w.get("name").and_then(Value::as_str).map(str::to_string),
            title: w.get("title").and_then(Value::as_str).map(str::to_string),
            tool_kind: w.get("kind").and_then(Value::as_str).map(str::to_string),
            arguments: w.get("rawInput").map(compact_json_string),
        }],
    }
}

/// Partial tool-call update wire → Start/Update (running) or End (terminal).
fn tool_call_update_events(w: &Value) -> Vec<HarnessEvent> {
    let Some(tool_call_id) = w.get("toolCallId").and_then(Value::as_str) else {
        return Vec::new();
    };
    let status = w.get("status").and_then(Value::as_str);
    let (content, file_changes) = content_of_tool_call(w.get("content"));
    let raw_output = raw_output_text(w);
    let content = content.or_else(|| raw_output.clone());
    match status {
        Some("completed") => vec![HarnessEvent::ToolCallEnd {
            tool_call_id: tool_call_id.to_string(),
            ok: true,
            content,
            error: None,
            file_changes,
        }],
        Some("failed") => vec![HarnessEvent::ToolCallEnd {
            tool_call_id: tool_call_id.to_string(),
            ok: false,
            content: content.clone(),
            error: content.or(Some("tool failed".to_string())),
            file_changes,
        }],
        Some(other) => vec![HarnessEvent::ToolCallUpdate {
            tool_call_id: tool_call_id.to_string(),
            status: Some(other.to_string()),
            content,
            raw_output,
        }],
        None => vec![HarnessEvent::ToolCallUpdate {
            tool_call_id: tool_call_id.to_string(),
            status: None,
            content,
            raw_output,
        }],
    }
}

/// Tool-call content array → (accumulated text, file changes from diff entries).
/// Wire: `[{"type":"content","content":{"type":"text","text":...}},
///         {"type":"diff","path":...,"oldText":...,"newText":...},
///         {"type":"terminal","terminalId":...}]`.
fn content_of_tool_call(content: Option<&Value>) -> (Option<String>, Vec<HarnessFileChange>) {
    let mut text = String::new();
    let mut changes = Vec::new();
    if let Some(arr) = content.and_then(Value::as_array) {
        for item in arr {
            match item.get("type").and_then(Value::as_str) {
                Some("content") => {
                    if let Some(t) = item.pointer("/content/text").and_then(Value::as_str) {
                        text.push_str(t);
                    }
                }
                Some("diff") => {
                    let path = item.get("path").and_then(Value::as_str).unwrap_or("");
                    let old = item.get("oldText").and_then(Value::as_str).unwrap_or("");
                    let new = item.get("newText").and_then(Value::as_str).unwrap_or("");
                    let name = std::path::Path::new(path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.to_string());
                    changes.push(HarnessFileChange {
                        file_path: path.to_string(),
                        action: "modify".to_string(),
                        diff: crate::utils::diff::simple_diff(old, new, &name),
                    });
                }
                _ => {}
            }
        }
    }
    let text = if text.is_empty() { None } else { Some(text) };
    (text, changes)
}

/// Text blocks from either ACP tool-call content (`{type:"content",
/// content:{type:"text"}}`) or omp's `rawOutput.content` (`{type:"text",
/// text}`). Agents keep human-readable tool output in one of the two; the raw
/// `details` object is machine state and is never shown verbatim.
fn text_blocks(value: Option<&Value>) -> Option<String> {
    let arr = value?.as_array()?;
    let mut out = String::new();
    for item in arr {
        let text = match item.get("type").and_then(Value::as_str) {
            Some("text") => item.get("text").and_then(Value::as_str),
            Some("content") => item.pointer("/content/text").and_then(Value::as_str),
            _ => None,
        };
        if let Some(t) = text {
            out.push_str(t);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// Human-readable text carried in a tool call's `rawOutput.content` (omp and
/// several adapters put the final tool text here instead of `content`).
fn raw_output_text(w: &Value) -> Option<String> {
    text_blocks(w.pointer("/rawOutput/content"))
}

/// `available_commands_update` wire → normalized command list.
fn commands_of(w: &Value) -> Vec<HarnessCommandInfo> {
    let Some(arr) = w.get("availableCommands").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|c| {
            Some(HarnessCommandInfo {
                name: c.get("name")?.as_str()?.to_string(),
                description: c
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                input_hint: c
                    .pointer("/input/hint")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        })
        .collect()
}

/// Structured subagent state carried by a tool call (omp `task` progress and
/// `wait` job snapshots). ACP has no subagent stream — agents that expose
/// subagents put this state in `rawOutput.details`; others expose none and the
/// event is simply absent.
fn subagent_update_event(w: &Value) -> Option<HarnessEvent> {
    let parent = w.get("toolCallId").and_then(Value::as_str)?.to_string();
    let mut entries: Vec<HarnessSubagentInfo> = Vec::new();
    if let Some(arr) = w
        .pointer("/rawOutput/details/progress")
        .and_then(Value::as_array)
    {
        for p in arr {
            let Some(id) = p.get("id").and_then(Value::as_str) else {
                continue;
            };
            entries.push(HarnessSubagentInfo {
                id: id.to_string(),
                tool_call_id: parent.clone(),
                agent: str_at(p, "agent"),
                title: None,
                task: p.get("task").and_then(Value::as_str).map(first_line),
                status: str_at(p, "status").unwrap_or_else(|| "running".to_string()),
                model: str_at(p, "resolvedModel"),
                thinking_level: str_at(p, "resolvedThinkingLevel"),
                duration_ms: p.get("durationMs").and_then(Value::as_u64),
                tokens: p.get("tokens").and_then(Value::as_u64),
                cost_usd: p.get("cost").and_then(Value::as_f64),
                recent_tools: strings_of(p.get("recentTools")),
                recent_output: strings_of(p.get("recentOutput")),
                result: None,
            });
        }
    }
    if let Some(arr) = w
        .pointer("/rawOutput/details/jobs")
        .and_then(Value::as_array)
    {
        for j in arr {
            let Some(id) = j.get("id").and_then(Value::as_str) else {
                continue;
            };
            // Background job snapshots also cover shells; only task jobs are subagents.
            if j.get("type").and_then(Value::as_str) != Some("task") {
                continue;
            }
            entries.push(HarnessSubagentInfo {
                id: id.to_string(),
                tool_call_id: parent.clone(),
                agent: str_at(j, "agent"),
                title: None,
                task: str_at(j, "label").map(|s| first_line(&s)),
                status: str_at(j, "status").unwrap_or_else(|| "running".to_string()),
                model: str_at(j, "resolvedModel"),
                thinking_level: str_at(j, "resolvedThinkingLevel"),
                duration_ms: j.get("durationMs").and_then(Value::as_u64),
                tokens: j.get("tokens").and_then(Value::as_u64),
                cost_usd: j.get("cost").and_then(Value::as_f64),
                recent_tools: Vec::new(),
                recent_output: strings_of(j.get("recentOutput")),
                result: j
                    .get("resultText")
                    .and_then(Value::as_str)
                    .map(|s| crate::utils::truncate::truncate_chars_head(s, 10000)),
            });
        }
    }
    (!entries.is_empty()).then_some(HarnessEvent::SubagentUpdate { entries })
}

fn str_at(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Compact string list from a string array or an array of objects
/// (`{toolName}`, `{name}`, `{label}` preferred). Only the newest entries are
/// kept and each is char-bounded — the UI shows one line each and these arrays
/// repeat on every progress update.
fn strings_of(v: Option<&Value>) -> Vec<String> {
    let Some(arr) = v.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out: Vec<String> = arr
        .iter()
        .filter_map(|item| {
            item.as_str().map(str::to_string).or_else(|| {
                ["toolName", "name", "label", "text", "summary"]
                    .iter()
                    .find_map(|k| item.get(*k).and_then(Value::as_str))
                    .map(str::to_string)
            })
        })
        .collect();
    if out.len() > 8 {
        out.drain(..out.len() - 8);
    }
    for item in &mut out {
        if item.chars().count() > 300 {
            let mut cut: String = item.chars().take(300).collect();
            cut.push('…');
            *item = cut;
        }
    }
    out
}

/// First non-empty line, char-bounded (subagent assignments are multi-line).
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    let mut out: String = line.chars().take(160).collect();
    if line.chars().count() > 160 {
        out.push('…');
    }
    out
}

/// Plan wire `{"entries":[{"content","priority","status"}, ...]}` → PlanUpdate.
fn plan_event(w: &Value) -> Option<HarnessEvent> {
    let arr = w.get("entries")?.as_array()?;
    let entries = arr
        .iter()
        .map(|e| HarnessPlanEntry {
            content: e
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            priority: e
                .get("priority")
                .and_then(Value::as_str)
                .unwrap_or("medium")
                .to_string(),
            status: e
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("pending")
                .to_string(),
        })
        .collect();
    Some(HarnessEvent::PlanUpdate { entries })
}

/// Usage wire `{"used":N,"size":N,"cost":{"amount":F,"currency":"USD"}}` → UsageUpdate.
fn usage_event(w: &Value) -> Option<HarnessEvent> {
    Some(HarnessEvent::UsageUpdate {
        usage: HarnessUsage {
            input_tokens: None,
            output_tokens: None,
            context_tokens: w.get("used").and_then(Value::as_u64),
            context_window: w.get("size").and_then(Value::as_u64),
            cost_usd: w.pointer("/cost/amount").and_then(Value::as_f64),
        },
    })
}

/// Notice wire `{"severity","title","description"}` → Notice (title + detail).
fn notice_event(w: &Value) -> Option<HarnessEvent> {
    let title = w.get("title").and_then(Value::as_str)?;
    let desc = w.get("description").and_then(Value::as_str);
    let message = match desc {
        Some(d) => format!("{title}: {d}"),
        None => title.to_string(),
    };
    Some(HarnessEvent::Notice { message })
}

/// Compute the spawn injections for the chosen spawn-option values: CLI args
/// and env vars, split by the option's inject kind. Values marked
/// `inject: false` (the tool default) and unknown ids inject nothing. Shared
/// by the ACP driver (spawn-time) and the Codex driver (per-turn).
pub(crate) fn spawn_option_injections(
    specs: &[SpawnOptionSpec],
    values: &HashMap<String, String>,
) -> (Vec<String>, Vec<(String, String)>) {
    let mut args = Vec::new();
    let mut env = Vec::new();
    for spec in specs {
        let Some(chosen) = values.get(&spec.option_id) else {
            continue;
        };
        let Some(v) = spec.values.iter().find(|v| &v.id == chosen) else {
            continue;
        };
        if !v.inject {
            continue;
        }
        match &spec.inject {
            SpawnOptionInject::Flag { arg } => {
                args.push(arg.clone());
                args.push(v.id.clone());
            }
            SpawnOptionInject::Env { var } => env.push((var.clone(), v.id.clone())),
        }
    }
    (args, env)
}

/// Synthesize one registry-declared spawn option into the normalized
/// config-option shape (category "spawn" marks it as spawn-time-only for the
/// GUI: persist + respawn instead of a live protocol change). Also used by the
/// `harness_tool_config_options` command for pre-spawn dropdown data.
pub(crate) fn synth_spawn_option(
    spec: &SpawnOptionSpec,
    current: Option<&String>,
) -> HarnessConfigOption {
    HarnessConfigOption {
        option_id: spec.option_id.clone(),
        name: Some(spec.name.clone()),
        kind: "select".to_string(),
        category: Some("spawn".to_string()),
        current: current
            .cloned()
            .or_else(|| spec.values.first().map(|v| v.id.clone())),
        values: spec
            .values
            .iter()
            .map(|v| HarnessConfigValue {
                id: v.id.clone(),
                name: Some(v.name.clone()),
            })
            .collect(),
        boolean_value: None,
    }
}

/// Parse the ACP `configOptions` array (session/new result or
/// config_option_update payload) into the normalized config-option model.
/// Returns None when the agent exposes nothing.
fn parse_acp_config_options(
    v: &Value,
) -> Option<Vec<crate::plugins::harness::model::HarnessConfigOption>> {
    // The array lives at /result/configOptions (session/new JSON-RPC response)
    // or /configOptions (config_option_update notification payload).
    let arr = v
        .pointer("/result/configOptions")
        .or_else(|| v.get("configOptions"))?
        .as_array()?;
    if arr.is_empty() {
        return None;
    }
    let options = arr
        .iter()
        .filter_map(parse_one_config_option)
        .collect::<Vec<_>>();
    (!options.is_empty()).then_some(options)
}

/// One ACP SessionConfigOption wire item → normalized HarnessConfigOption.
/// Wire shapes (schema-verified): select → `{id, name, category?, type:"select",
/// currentValue, options: [{value, name}] | [{name, options: [...]}]}`;
/// boolean → `{id, name, category?, type:"boolean", currentValue: bool}`.
fn parse_one_config_option(
    item: &Value,
) -> Option<crate::plugins::harness::model::HarnessConfigOption> {
    use crate::plugins::harness::model::HarnessConfigOption;
    let option_id = item.get("id")?.as_str()?.to_string();
    let name = item.get("name").and_then(Value::as_str).map(str::to_string);
    let category = item
        .get("category")
        .and_then(Value::as_str)
        .map(str::to_string);
    match item.get("type").and_then(Value::as_str) {
        Some("boolean") => Some(HarnessConfigOption {
            option_id,
            name,
            kind: "boolean".to_string(),
            category,
            current: None,
            values: Vec::new(),
            boolean_value: item.get("currentValue").and_then(Value::as_bool),
        }),
        _ => {
            // select (the default kind)
            let current = item
                .get("currentValue")
                .and_then(Value::as_str)
                .map(str::to_string);
            let values = parse_config_select_values(item.get("options"));
            Some(HarnessConfigOption {
                option_id,
                name,
                kind: "select".to_string(),
                category,
                current,
                values,
                boolean_value: None,
            })
        }
    }
}

/// Parse select-option values; handles both the flat list
/// (`[{value, name}, ...]`) and the grouped list (`[{name, options: [...]}]`).
fn parse_config_select_values(
    options: Option<&Value>,
) -> Vec<crate::plugins::harness::model::HarnessConfigValue> {
    use crate::plugins::harness::model::HarnessConfigValue;
    let Some(arr) = options.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in arr {
        if let Some(value) = entry.get("value").and_then(Value::as_str) {
            // Flat entry: {value, name}
            out.push(HarnessConfigValue {
                id: value.to_string(),
                name: entry
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            });
        } else if let Some(group) = entry.get("options").and_then(Value::as_array) {
            // Grouped entry: {name: "Group label", options: [{value, name}, ...]}
            let group_label = entry.get("name").and_then(Value::as_str);
            for sub in group {
                if let Some(value) = sub.get("value").and_then(Value::as_str) {
                    out.push(HarnessConfigValue {
                        id: value.to_string(),
                        name: sub
                            .get("name")
                            .and_then(Value::as_str)
                            .or(group_label)
                            .map(str::to_string),
                    });
                }
            }
        }
    }
    out
}

/// First diff entry of a permission toolCall → unified diff preview text.
fn preview_diff_from_tool_call(wire: &Value) -> Option<String> {
    let arr = wire.pointer("/toolCall/content")?.as_array()?;
    for item in arr {
        if item.get("type").and_then(Value::as_str) == Some("diff") {
            let path = item.get("path").and_then(Value::as_str).unwrap_or("");
            let old = item.get("oldText").and_then(Value::as_str).unwrap_or("");
            let new = item.get("newText").and_then(Value::as_str).unwrap_or("");
            let name = std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string());
            return Some(crate::utils::diff::simple_diff(old, new, &name));
        }
    }
    None
}

/// File path of the permission toolCall's first diff entry (DiffView label).
fn preview_file_path(wire: &Value) -> Option<String> {
    let arr = wire.pointer("/toolCall/content")?.as_array()?;
    for item in arr {
        if item.get("type").and_then(Value::as_str) == Some("diff") {
            return item.get("path").and_then(Value::as_str).map(str::to_string);
        }
    }
    None
}

/// Compact JSON for display (strings pass through; objects pretty-compact).
fn compact_json_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Config options: session/new-style response (result.configOptions) and
    /// config_option_update notification both parse into the normalized model;
    /// grouped select lists flatten; booleans pass through.
    #[test]
    fn parses_config_options_select_and_boolean() {
        let resp = json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {
                "sessionId": "sess_1",
                "configOptions": [
                    {
                        "id": "model",
                        "name": "Model",
                        "category": "model",
                        "type": "select",
                        "currentValue": "opus",
                        "options": [
                            { "value": "sonnet", "name": "Sonnet" },
                            { "value": "opus", "name": "Opus" }
                        ]
                    },
                    {
                        "id": "brave_mode",
                        "name": "Brave Mode",
                        "type": "boolean",
                        "currentValue": true
                    }
                ]
            }
        });
        let options = parse_acp_config_options(&resp).expect("options present");
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].option_id, "model");
        assert_eq!(options[0].kind, "select");
        assert_eq!(options[0].category.as_deref(), Some("model"));
        assert_eq!(options[0].current.as_deref(), Some("opus"));
        assert_eq!(options[0].values.len(), 2);
        assert_eq!(options[0].values[0].id, "sonnet");
        assert_eq!(options[0].values[1].name.as_deref(), Some("Opus"));
        assert_eq!(options[1].option_id, "brave_mode");
        assert_eq!(options[1].kind, "boolean");
        assert_eq!(options[1].boolean_value, Some(true));

        // config_option_update notification payload (top-level configOptions).
        let update = json!({
            "sessionUpdate": "config_option_update",
            "configOptions": [
                {
                    "id": "reasoning_effort",
                    "name": "Reasoning Effort",
                    "category": "thought_level",
                    "type": "select",
                    "currentValue": "high",
                    "options": [
                        { "name": "Provider A", "options": [
                            { "value": "low", "name": "Low" },
                            { "value": "high", "name": "High" }
                        ]}
                    ]
                }
            ]
        });
        let options = parse_acp_config_options(&update).expect("options present");
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].option_id, "reasoning_effort");
        assert_eq!(options[0].current.as_deref(), Some("high"));
        // Grouped list flattened.
        assert_eq!(options[0].values.len(), 2);
        assert_eq!(options[0].values[1].id, "high");
        assert_eq!(options[0].values[1].name.as_deref(), Some("High"));

        // Empty/absent option sets → None.
        assert!(parse_acp_config_options(&json!({ "result": {} })).is_none());
        assert!(parse_acp_config_options(&json!({})).is_none());
    }

    /// The config_option_update SessionUpdate maps to a ConfigOptions event
    /// end-to-end through map_session_update.
    #[test]
    fn maps_config_option_update_event() {
        let wire = json!({
            "sessionId": "sess_a",
            "update": {
                "sessionUpdate": "config_option_update",
                "configOptions": [
                    {
                        "id": "model",
                        "name": "Model",
                        "type": "select",
                        "currentValue": "m2",
                        "options": [ { "value": "m1", "name": "M1" }, { "value": "m2", "name": "M2" } ]
                    }
                ]
            }
        });
        let note: SessionNotification = serde_json::from_value(wire).unwrap();
        let events = map_session_update("t", note.update);
        match &events[..] {
            [HarnessEvent::ConfigOptions { options }] => {
                assert_eq!(options[0].option_id, "model");
                assert_eq!(options[0].current.as_deref(), Some("m2"));
            }
            other => panic!("expected one ConfigOptions event, got {other:?}"),
        }
    }

    /// Spawn options (CLI-flag/env-backed): chosen injectable values become
    /// flag argv pairs or env vars by inject kind; the default value and
    /// unknown ids inject nothing; synthesis marks the category "spawn" with
    /// the chosen (or first) current.
    #[test]
    fn spawn_option_args_and_synthesis() {
        use crate::plugins::harness::registry::{
            SpawnOptionInject, SpawnOptionSpec, SpawnOptionValueSpec,
        };
        let flag_spec = SpawnOptionSpec {
            option_id: "approval-mode".to_string(),
            name: "Approval Mode".to_string(),
            inject: SpawnOptionInject::Flag {
                arg: "--approval-mode".to_string(),
            },
            values: vec![
                SpawnOptionValueSpec {
                    id: "always-ask".to_string(),
                    name: "Always ask".to_string(),
                    inject: false,
                },
                SpawnOptionValueSpec {
                    id: "write".to_string(),
                    name: "Write".to_string(),
                    inject: true,
                },
                SpawnOptionValueSpec {
                    id: "yolo".to_string(),
                    name: "YOLO".to_string(),
                    inject: true,
                },
            ],
        };
        let env_spec = SpawnOptionSpec {
            option_id: "goose-mode".to_string(),
            name: "Permission Mode".to_string(),
            inject: SpawnOptionInject::Env {
                var: "GOOSE_MODE".to_string(),
            },
            values: vec![
                SpawnOptionValueSpec {
                    id: "auto".to_string(),
                    name: "Autonomous".to_string(),
                    inject: false,
                },
                SpawnOptionValueSpec {
                    id: "approve".to_string(),
                    name: "Manual approval".to_string(),
                    inject: true,
                },
            ],
        };
        let specs = vec![flag_spec.clone(), env_spec];

        // Default value (inject=false) → no injections.
        let mut values = HashMap::from([("approval-mode".to_string(), "always-ask".to_string())]);
        assert_eq!(spawn_option_injections(&specs, &values), (vec![], vec![]));
        // Injectable flag values → two argv entries (space-separated form).
        values.insert("approval-mode".to_string(), "yolo".to_string());
        assert_eq!(
            spawn_option_injections(&specs, &values),
            (
                vec!["--approval-mode".to_string(), "yolo".to_string()],
                vec![]
            )
        );
        // Env-kind option → env var, no args.
        values.insert("goose-mode".to_string(), "approve".to_string());
        assert_eq!(
            spawn_option_injections(&specs, &values),
            (
                vec!["--approval-mode".to_string(), "yolo".to_string()],
                vec![("GOOSE_MODE".to_string(), "approve".to_string())]
            )
        );
        // Unknown value id → ignored.
        values.insert("approval-mode".to_string(), "bogus".to_string());
        assert_eq!(
            spawn_option_injections(&specs, &values),
            (
                vec![],
                vec![("GOOSE_MODE".to_string(), "approve".to_string())]
            )
        );

        // Synthesis: current = chosen value, category "spawn", full value list.
        let opt = synth_spawn_option(&flag_spec, values.get("approval-mode"));
        assert_eq!(opt.option_id, "approval-mode");
        assert_eq!(opt.category.as_deref(), Some("spawn"));
        assert_eq!(opt.current.as_deref(), Some("bogus"));
        assert_eq!(opt.values.len(), 3);
        // Synthesis without a preference falls back to the first value.
        let opt = synth_spawn_option(&flag_spec, None);
        assert_eq!(opt.current.as_deref(), Some("always-ask"));
    }

    /// ACP command menu: available_commands_update carries name/description and
    /// an optional input hint; it maps to the frontend slash-command list.
    #[test]
    fn maps_available_commands() {
        let note: SessionNotification = serde_json::from_value(json!({
            "sessionId": "s",
            "update": {
                "sessionUpdate": "available_commands_update",
                "availableCommands": [
                    { "name": "model", "description": "Show current model selection" },
                    { "name": "switch", "description": "Switch model", "input": { "hint": "[model]" } }
                ]
            }
        }))
        .unwrap();
        match map_session_update("t", note.update).pop().unwrap() {
            HarnessEvent::CommandsUpdate { commands } => {
                assert_eq!(commands.len(), 2);
                assert_eq!(commands[0].name, "model");
                assert_eq!(commands[0].description, "Show current model selection");
                assert!(commands[0].input_hint.is_none());
                assert_eq!(commands[1].input_hint.as_deref(), Some("[model]"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    /// session/load replays user messages as user_message_chunk; the history
    /// builder needs them to delimit turns.
    #[test]
    fn maps_user_message_chunk() {
        let note: SessionNotification = serde_json::from_value(json!({
            "sessionId": "s",
            "update": {
                "sessionUpdate": "user_message_chunk",
                "content": { "type": "text", "text": "你好" }
            }
        }))
        .unwrap();
        match map_session_update("t", note.update).pop().unwrap() {
            HarnessEvent::UserTextDelta { text } => assert_eq!(text, "你好"),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    /// omp puts tool text in `rawOutput.content` (not `content`) and exposes
    /// subagent progress in `rawOutput.details.progress`. Both must surface:
    /// the text as the tool result, the progress as a SubagentUpdate.
    #[test]
    fn maps_omp_task_progress_and_raw_output_text() {
        let note: SessionNotification = serde_json::from_value(json!({
            "sessionId": "s",
            "update": {
                "sessionUpdate": "tool_call_update",
                "toolCallId": "call_task",
                "status": "in_progress",
                "rawOutput": {
                    "content": [ { "type": "text", "text": "Running agent Foo..." } ],
                    "details": {
                        "progress": [{
                            "index": 0,
                            "id": "Foo",
                            "agent": "sonic",
                            "agentSource": "bundled",
                            "modelRole": "smol",
                            "status": "running",
                            "task": "# Target\nDo the thing\n\n# Change\nDo it",
                            "assignment": "Do it",
                            "recentTools": ["read_file"],
                            "recentOutput": ["half"],
                            "toolCount": 1,
                            "requests": 2,
                            "tokens": 1234,
                            "cost": 0.002,
                            "durationMs": 1700,
                            "resolvedModel": "scnet/model",
                            "resolvedThinkingLevel": "high"
                        }]
                    }
                }
            }
        }))
        .unwrap();
        let events = map_session_update("t", note.update);
        let tool = events
            .iter()
            .find(|e| matches!(e, HarnessEvent::ToolCallUpdate { .. }))
            .expect("tool update event");
        match tool {
            HarnessEvent::ToolCallUpdate { content, .. } => {
                assert_eq!(content.as_deref(), Some("Running agent Foo..."));
            }
            _ => unreachable!(),
        }
        let subagents = events
            .iter()
            .find(|e| matches!(e, HarnessEvent::SubagentUpdate { .. }))
            .expect("subagent update event");
        match subagents {
            HarnessEvent::SubagentUpdate { entries } => {
                assert_eq!(entries.len(), 1);
                let s = &entries[0];
                assert_eq!(s.id, "Foo");
                assert_eq!(s.agent.as_deref(), Some("sonic"));
                assert_eq!(s.status, "running");
                assert_eq!(s.task.as_deref(), Some("# Target"));
                assert_eq!(s.model.as_deref(), Some("scnet/model"));
                assert_eq!(s.thinking_level.as_deref(), Some("high"));
                assert_eq!(s.duration_ms, Some(1700));
                assert_eq!(s.tokens, Some(1234));
                assert_eq!(s.recent_tools, vec!["read_file"]);
                assert_eq!(s.recent_output, vec!["half"]);
            }
            _ => unreachable!(),
        }
    }

    /// Replayed events collapse into user/assistant messages with the same
    /// agentSteps JSON the live frontend assembles.
    #[test]
    fn builds_history_messages_from_replay() {
        let events = vec![
            HarnessEvent::UserTextDelta {
                text: "hi".to_string(),
            },
            HarnessEvent::ThinkingDelta {
                text: "think".to_string(),
            },
            HarnessEvent::TextDelta {
                text: "hello".to_string(),
            },
            HarnessEvent::ToolCallStart {
                tool_call_id: "call_1".to_string(),
                name: Some("read_file".to_string()),
                title: None,
                tool_kind: Some("read".to_string()),
                arguments: Some("{\"path\":\"a\"}".to_string()),
            },
            HarnessEvent::ToolCallEnd {
                tool_call_id: "call_1".to_string(),
                ok: true,
                content: Some("ok".to_string()),
                error: None,
                file_changes: Vec::new(),
            },
            HarnessEvent::SubagentUpdate {
                entries: vec![HarnessSubagentInfo {
                    id: "Foo".to_string(),
                    tool_call_id: "call_task".to_string(),
                    agent: Some("sonic".to_string()),
                    title: None,
                    task: Some("Do it".to_string()),
                    status: "completed".to_string(),
                    model: None,
                    thinking_level: None,
                    duration_ms: Some(1500),
                    tokens: Some(42),
                    cost_usd: None,
                    recent_tools: vec!["read_file".to_string()],
                    recent_output: vec!["done".to_string()],
                    result: Some("BANANA".to_string()),
                }],
            },
            HarnessEvent::UsageUpdate {
                usage: HarnessUsage {
                    context_tokens: Some(5000),
                    ..Default::default()
                },
            },
            HarnessEvent::TurnEnded {
                stop_reason: "end_turn".to_string(),
                error: None,
            },
        ];
        let messages = build_history(&events);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, json!("hi"));
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, json!("hello"));
        assert_eq!(messages[1].reasoning.as_deref(), Some("think"));
        assert_eq!(messages[1].context_tokens, Some(5000));
        let steps = messages[1].agent_steps.as_ref().expect("steps");
        let steps = steps.as_array().unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0]["type"], "tool_call");
        assert_eq!(steps[0]["status"], "complete");
        assert_eq!(steps[0]["toolCall"]["content"], "ok");
        assert_eq!(steps[1]["type"], "subagent");
        assert_eq!(steps[1]["status"], "complete");
        assert_eq!(steps[1]["subagentId"], "Foo");
        assert_eq!(steps[1]["subagentResult"], "BANANA");
        assert_eq!(steps[1]["duration"], 1500);
    }

    /// Literal wire examples from the ACP v1 spec/research: the mapping must
    /// produce exactly the normalized events the frontend expects.
    #[test]
    fn maps_agent_message_and_thought_chunks() {
        let wire = json!({
            "sessionId": "sess_a",
            "update": {
                "sessionUpdate": "agent_message_chunk",
                "messageId": "msg_1",
                "content": { "type": "text", "text": "Hello " }
            }
        });
        let note: SessionNotification = serde_json::from_value(wire).unwrap();
        let events = map_session_update("t", note.update);
        assert!(matches!(
            &events[..],
            [HarnessEvent::TextDelta { text }] if text == "Hello "
        ));

        let wire = json!({
            "sessionId": "sess_a",
            "update": {
                "sessionUpdate": "agent_thought_chunk",
                "content": { "type": "text", "text": "thinking..." }
            }
        });
        let note: SessionNotification = serde_json::from_value(wire).unwrap();
        let events = map_session_update("t", note.update);
        assert!(
            matches!(&events[..], [HarnessEvent::ThinkingDelta { text }] if text == "thinking...")
        );
    }

    #[test]
    fn maps_tool_call_lifecycle() {
        let start: SessionNotification = serde_json::from_value(json!({
            "sessionId": "s",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "call_1",
                "title": "Reading configuration file",
                "name": "read_file",
                "kind": "read",
                "status": "pending"
            }
        }))
        .unwrap();
        match map_session_update("t", start.update).pop().unwrap() {
            HarnessEvent::ToolCallStart {
                tool_call_id, name, ..
            } => {
                assert_eq!(tool_call_id, "call_1");
                assert_eq!(name.as_deref(), Some("read_file"));
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let done: SessionNotification = serde_json::from_value(json!({
            "sessionId": "s",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "call_1",
                "title": "Reading configuration file",
                "kind": "read",
                "status": "completed",
                "content": [ { "type": "content", "content": { "type": "text", "text": "ok" } } ]
            }
        }))
        .unwrap();
        match map_session_update("t", done.update).pop().unwrap() {
            HarnessEvent::ToolCallEnd {
                tool_call_id,
                ok,
                content,
                ..
            } => {
                assert_eq!(tool_call_id, "call_1");
                assert!(ok);
                assert_eq!(content.as_deref(), Some("ok"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn maps_diff_content_into_file_changes() {
        let ev: SessionNotification = serde_json::from_value(json!({
            "sessionId": "s",
            "update": {
                "sessionUpdate": "tool_call",
                "toolCallId": "c",
                "title": "Editing",
                "kind": "edit",
                "status": "completed",
                "content": [ {
                    "type": "diff",
                    "path": "/home/u/proj/src/main.rs",
                    "oldText": "fn a() {}",
                    "newText": "fn b() {}"
                } ]
            }
        }))
        .unwrap();
        match map_session_update("t", ev.update).pop().unwrap() {
            HarnessEvent::ToolCallEnd { file_changes, .. } => {
                assert_eq!(file_changes.len(), 1);
                assert_eq!(file_changes[0].file_path, "/home/u/proj/src/main.rs");
                assert!(file_changes[0].diff.contains("fn a"));
                assert!(file_changes[0].diff.contains("fn b"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn maps_plan_and_usage() {
        let plan: SessionNotification = serde_json::from_value(json!({
            "sessionId": "s",
            "update": {
                "sessionUpdate": "plan",
                "entries": [
                    { "content": "Check for syntax errors", "priority": "high", "status": "pending" }
                ]
            }
        }))
        .unwrap();
        match map_session_update("t", plan.update).pop().unwrap() {
            HarnessEvent::PlanUpdate { entries } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(entries[0].priority, "high");
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let usage: SessionNotification = serde_json::from_value(json!({
            "sessionId": "s",
            "update": {
                "sessionUpdate": "usage_update",
                "used": 4200,
                "size": 200000,
                "cost": { "amount": 0.0234, "currency": "USD" }
            }
        }))
        .unwrap();
        match map_session_update("t", usage.update).pop().unwrap() {
            HarnessEvent::UsageUpdate { usage } => {
                assert_eq!(usage.context_tokens, Some(4200));
                assert_eq!(usage.context_window, Some(200000));
                assert_eq!(usage.cost_usd, Some(0.0234));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    /// Unknown future variants must be skipped, not crash the driver.
    #[test]
    fn unknown_update_variants_are_skipped() {
        let v = json!({
            "sessionId": "s",
            "update": { "sessionUpdate": "some_future_variant", "x": 1 }
        });
        assert!(serde_json::from_value::<SessionNotification>(v).is_err());
    }

    /// Permission requests parse via the crate type; option kinds arrive intact.
    #[test]
    fn parses_permission_request() {
        let req: RequestPermissionRequest = serde_json::from_value(json!({
            "sessionId": "sess_abc",
            "toolCall": {
                "toolCallId": "call_001",
                "title": "Running command",
                "kind": "execute",
                "status": "pending"
            },
            "options": [
                { "optionId": "allow-once", "name": "Allow once", "kind": "allow_once" },
                { "optionId": "reject-once", "name": "Reject", "kind": "reject_once" }
            ]
        }))
        .unwrap();
        let wire = serde_json::to_value(&req).unwrap();
        assert_eq!(wire.pointer("/toolCall/title").unwrap(), "Running command");
        let opts = wire.get("options").unwrap().as_array().unwrap();
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].get("kind").unwrap(), "allow_once");
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::plugins::harness::adapter::{
        kill_child_tree, DriverCtx, DriverSpawn, HarnessCommand, HarnessEmitter,
    };
    use crate::plugins::harness::model::{DriverEvent, HarnessEvent};
    use crate::plugins::harness::registry;
    use crate::plugins::harness::store;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    /// In-memory emitter for the live test: persists session ids into the test
    /// connection so resume bookkeeping is verifiable without a Tauri runtime.
    struct TestEmitter {
        conn: parking_lot::Mutex<rusqlite::Connection>,
    }

    impl HarnessEmitter for TestEmitter {
        fn emit_approval_requested(
            &self,
            _tool_id: &str,
            _session_row_id: &str,
            _request: crate::plugins::harness::model::HarnessApprovalRequest,
        ) {
        }
        fn emit_approval_expired(&self, _request_id: &str) {}
        fn set_harness_session_id(&self, session_row_id: &str, harness_session_id: &str) {
            store::set_session_harness_id(&self.conn.lock(), session_row_id, harness_session_id)
                .expect("persist harness session id");
        }
        fn replace_history(
            &self,
            session_row_id: &str,
            messages: &[crate::core::models::chat::Message],
        ) {
            let mut conn = self.conn.lock();
            store::replace_messages(&mut conn, session_row_id, messages).expect("replace history");
        }
    }

    /// Live roundtrip against a locally installed ACP agent: spawn, initialize,
    /// session/new, one full prompt turn (stream + turn end), session-id persist.
    ///
    /// Ignored by default — it drives a REAL CLI agent process which needs the
    /// tool on PATH with valid credentials (dsh ships an ACP profile and is the
    /// zero-install agent on this dev machine). Run explicitly:
    /// `cargo test --manifest-path src-tauri/Cargo.toml acp_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "live: requires `dsh` on PATH with configured credentials"]
    async fn acp_live_roundtrip_dsh() {
        if crate::utils::exec_resolve::resolve_executable("dsh").is_none() {
            eprintln!("[live] dsh 未安装，跳过真实联调");
            return;
        }
        // Workspace: fresh temp dir (absolute path WITHOUT the Windows verbatim
        // prefix — third-party CLIs choke on `\\?\` paths, see store::canonical_workspace_path).
        let ws = std::env::temp_dir().join(format!("aio-harness-live-{}", std::process::id()));
        std::fs::create_dir_all(&ws).unwrap();
        let workspace_path = crate::plugins::harness::store::canonical_workspace_path(&ws).unwrap();

        // In-memory DB with the real base schema + migration table + one session row.
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::core::db::create_base_schema(&conn).unwrap();
        crate::core::migrate::run_migrations(&conn, crate::core::db::CHAT_DB_MIGRATIONS).unwrap();
        const ROW_ID: &str = "live-test-row";
        conn.execute(
            "INSERT INTO harness_sessions (id, tool_id, workspace_path, title, created_at, updated_at)
             VALUES (?1, 'dsh', ?2, 'live test', 'now', 'now')",
            rusqlite::params![ROW_ID, workspace_path],
        )
        .unwrap();

        let token = tokio_util::sync::CancellationToken::new();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<DriverEvent>();
        let ctx = DriverCtx {
            emitter: Arc::new(TestEmitter {
                conn: parking_lot::Mutex::new(conn),
            }),
            client_version: "test".to_string(),
            key: ("dsh".to_string(), ROW_ID.to_string()),
            token: token.clone(),
            events: event_tx,
            approvals: Arc::new(dashmap::DashMap::new()),
        };

        let spec = registry::lookup("dsh").expect("dsh 必须在注册表内");
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<HarnessCommand>();
        let handle = AcpDriver::launch(
            DriverSpawn {
                ctx,
                spec,
                workspace_path: workspace_path.clone(),
                harness_session_id: None,
                permission_mode: None,
                initial_model: None,
                initial_thinking: None,
                spawn_option_values: Default::default(),
                replay_history: false,
                extra_env: vec![],
            },
            cmd_rx,
        )
        .await
        .expect("ACP 握手 + 会话建立");

        // Full prompt turn: collect events until TurnEnded (generous live budget).
        cmd_tx
            .send(HarnessCommand::Prompt {
                text: "Reply with exactly the single word: hello".to_string(),
            })
            .unwrap();
        let mut got_text = false;
        let mut harness_sid: Option<String> = None;
        let mut turn_error: Option<String> = None;
        let ended = tokio::time::timeout(std::time::Duration::from_secs(240), async {
            loop {
                match event_rx.recv().await.expect("driver event channel") {
                    DriverEvent::State { state, .. } => {
                        eprintln!("[live] state: {state}");
                    }
                    DriverEvent::Event(HarnessEvent::SessionInfo {
                        harness_session_id, ..
                    }) => {
                        harness_sid = harness_session_id;
                    }
                    DriverEvent::Event(HarnessEvent::TextDelta { text }) => {
                        eprintln!("[live] text: {text}");
                        if !text.trim().is_empty() {
                            got_text = true;
                        }
                    }
                    DriverEvent::Event(HarnessEvent::ThinkingDelta { text }) => {
                        eprintln!("[live] think: {text}");
                    }
                    DriverEvent::Event(HarnessEvent::Error { message }) => {
                        turn_error = Some(message);
                    }
                    DriverEvent::Event(HarnessEvent::TurnEnded { stop_reason, error }) => {
                        eprintln!("[live] turn ended: {stop_reason} ({error:?})");
                        break;
                    }
                    DriverEvent::Event(other) => {
                        eprintln!(
                            "[live] event: {}",
                            serde_json::to_string(&other).unwrap_or_default()
                        );
                    }
                }
            }
        })
        .await;

        // Cleanup: cancel + hard-kill the agent process tree before asserting.
        token.cancel();
        if let Some(child) = &handle.child {
            kill_child_tree(child).await;
        }
        let _ = std::fs::remove_dir_all(&ws);

        assert!(ended.is_ok(), "240s 内未收到 TurnEnded");
        assert!(turn_error.is_none(), "驱动器报错: {}", turn_error.unwrap());
        assert!(got_text, "回合内未收到任何文本流");
        assert!(harness_sid.is_some(), "握手未上报 harness 会话 ID");
    }

    /// Live `session/load` roundtrip against omp: run one real turn, kill the
    /// process, then relaunch with `replay_history` — the agent's transcript
    /// must arrive as one History snapshot (user + assistant) and the command
    /// menu must be advertised. This is the regression the GUI history fix is
    /// built on.
    ///
    /// Ignored by default: needs `omp` on PATH with configured credentials.
    /// `cargo test --manifest-path src-tauri/Cargo.toml acp_live_omp -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "live: requires `omp` on PATH with configured credentials"]
    async fn acp_live_omp_history_replay() {
        if crate::utils::exec_resolve::resolve_executable("omp").is_none() {
            eprintln!("[live] omp 未安装，跳过真实联调");
            return;
        }
        let ws = std::env::temp_dir().join(format!("aio-omp-replay-{}", std::process::id()));
        std::fs::create_dir_all(&ws).unwrap();
        let workspace_path = store::canonical_workspace_path(&ws).unwrap();
        const ROW_ID: &str = "omp-replay-row";

        let live_conn = || {
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            crate::core::db::create_base_schema(&conn).unwrap();
            crate::core::migrate::run_migrations(&conn, crate::core::db::CHAT_DB_MIGRATIONS)
                .unwrap();
            conn.execute(
                "INSERT INTO harness_sessions (id, tool_id, workspace_path, title, created_at, updated_at)
                 VALUES (?1, 'omp', ?2, 'live replay', 'now', 'now')",
                rusqlite::params![ROW_ID, workspace_path],
            )
            .unwrap();
            conn
        };

        // ---- Turn 1: fresh session + one real prompt ----
        let token = tokio_util::sync::CancellationToken::new();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<DriverEvent>();
        let ctx = DriverCtx {
            emitter: Arc::new(TestEmitter {
                conn: parking_lot::Mutex::new(live_conn()),
            }),
            client_version: "test".to_string(),
            key: ("omp".to_string(), ROW_ID.to_string()),
            token: token.clone(),
            events: event_tx,
            approvals: Arc::new(dashmap::DashMap::new()),
        };
        let spec = registry::lookup("omp").expect("omp 必须在注册表内");
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<HarnessCommand>();
        let handle = AcpDriver::launch(
            DriverSpawn {
                ctx,
                spec: spec.clone(),
                workspace_path: workspace_path.clone(),
                harness_session_id: None,
                permission_mode: None,
                initial_model: None,
                initial_thinking: None,
                spawn_option_values: Default::default(),
                replay_history: false,
                extra_env: vec![],
            },
            cmd_rx,
        )
        .await
        .expect("ACP 握手 + 会话建立");
        cmd_tx
            .send(HarnessCommand::Prompt {
                text: "Reply with exactly the single word: hello".to_string(),
            })
            .unwrap();
        let mut harness_sid: Option<String> = None;
        let turn = tokio::time::timeout(std::time::Duration::from_secs(240), async {
            loop {
                match event_rx.recv().await.expect("driver event channel") {
                    DriverEvent::Event(HarnessEvent::SessionInfo {
                        harness_session_id, ..
                    }) => harness_sid = harness_session_id,
                    DriverEvent::Event(HarnessEvent::TurnEnded { .. }) => break,
                    _ => {}
                }
            }
        })
        .await;
        token.cancel();
        if let Some(child) = &handle.child {
            kill_child_tree(child).await;
        }
        assert!(turn.is_ok(), "首轮 240s 内未结束");
        let harness_sid = harness_sid.expect("首轮未上报会话 ID");

        // ---- Turn 2: replay the same session (no prompt) ----
        let token2 = tokio_util::sync::CancellationToken::new();
        let (event_tx2, mut event_rx2) = mpsc::unbounded_channel::<DriverEvent>();
        let ctx2 = DriverCtx {
            emitter: Arc::new(TestEmitter {
                conn: parking_lot::Mutex::new(live_conn()),
            }),
            client_version: "test".to_string(),
            key: ("omp".to_string(), ROW_ID.to_string()),
            token: token2.clone(),
            events: event_tx2,
            approvals: Arc::new(dashmap::DashMap::new()),
        };
        let (cmd_tx2, cmd_rx2) = mpsc::unbounded_channel::<HarnessCommand>();
        let handle2 = AcpDriver::launch(
            DriverSpawn {
                ctx: ctx2,
                spec,
                workspace_path: workspace_path.clone(),
                harness_session_id: Some(harness_sid),
                permission_mode: None,
                initial_model: None,
                initial_thinking: None,
                spawn_option_values: Default::default(),
                replay_history: true,
                extra_env: vec![],
            },
            cmd_rx2,
        )
        .await
        .expect("session/load 建立会话");
        let mut history: Option<Vec<crate::core::models::chat::Message>> = None;
        let mut commands = 0usize;
        let loaded = tokio::time::timeout(std::time::Duration::from_secs(60), async {
            loop {
                match event_rx2.recv().await.expect("driver event channel") {
                    DriverEvent::Event(HarnessEvent::History { messages }) => {
                        history = Some(messages)
                    }
                    DriverEvent::Event(HarnessEvent::CommandsUpdate { commands: list }) => {
                        commands = list.len()
                    }
                    DriverEvent::State { state, .. } if state == "idle" => break,
                    _ => {}
                }
            }
        })
        .await;
        assert!(loaded.is_ok(), "60s 内未完成 session/load");
        let messages = history.expect("session/load 未产生 History");
        assert!(messages.len() >= 2, "历史应含用户与助手消息");
        assert_eq!(messages[0].role, "user");
        assert!(messages.iter().any(|m| m.role == "assistant"));
        // omp sends available_commands_update right after the load response, so
        // it may land just after the idle state — wait briefly before asserting.
        if commands == 0 {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while let Some(ev) = event_rx2.recv().await {
                    if let DriverEvent::Event(HarnessEvent::CommandsUpdate { commands: list }) = ev
                    {
                        if !list.is_empty() {
                            commands = list.len();
                            break;
                        }
                    }
                }
            })
            .await;
        }
        token2.cancel();
        if let Some(child) = &handle2.child {
            kill_child_tree(child).await;
        }
        let _ = std::fs::remove_dir_all(&ws);
        assert!(commands > 0, "omp 应上报斜杠命令");
        let _ = cmd_tx2;
    }
}
