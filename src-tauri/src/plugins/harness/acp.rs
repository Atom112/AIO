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

use crate::plugins::harness::adapter::{
    resolve_acp_command, spawn_harness_child, DriverCtx, DriverHandle, DriverSpawn, HarnessAdapter,
    HarnessCommand,
};
use crate::plugins::harness::model::{
    HarnessApprovalOption, HarnessApprovalRequest, HarnessConfigOption, HarnessConfigValue,
    HarnessEvent, HarnessFileChange, HarnessPlanEntry, HarnessUsage,
};
use crate::plugins::harness::registry::SpawnOptionSpec;
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
        // Registry-declared CLI-flag-backed options (omp --approval-mode etc.):
        // inject the chosen value's flag at process start.
        args.extend(spawn_option_args(
            &spawn.spec.spawn_options,
            &spawn.spawn_option_values,
        ));
        let mut child = spawn_harness_child(&program, &args, &spawn.workspace_path)?;
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
                    "clientCapabilities": {},
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

        // session/resume (reattach without replay — AIO's own DB is the display
        // history) with fallback to a fresh session/new when the agent never saw
        // the id or lacks the resume capability.
        let mut harness_sid: Option<String> = None;
        let mut session_resp: Option<Value> = None;
        if let Some(rid) = spawn.harness_session_id.clone() {
            let Ok((resume_id, resume_rx)) = driver
                .send_request(
                    "session/resume",
                    json!({ "sessionId": rid, "cwd": spawn.workspace_path, "mcpServers": [] }),
                )
                .await
            else {
                return Err(format!(
                    "[{tool_label}] 发送 session/resume 失败：agent 进程已退出"
                ));
            };
            match driver
                .wait_request(resume_id, resume_rx, SESSION_SETUP_TIMEOUT_SECS)
                .await
            {
                Ok(v) => {
                    harness_sid = Some(rid);
                    session_resp = Some(v);
                }
                Err(e) => {
                    tracing::info!("[harness:{tool_label}] resume {rid} 失败（回退新会话）: {e}");
                }
            }
        }
        if harness_sid.is_none() {
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
    fn handle_session_update(&self, params: Option<&Value>) {
        let Some(params) = params else { return };
        match serde_json::from_value::<SessionNotification>(params.clone()) {
            Ok(note) => {
                for event in map_session_update(&self.tool_label, note.update) {
                    // Agent-reported config options: store the set and emit the
                    // merged view (agent options + synthesized spawn options).
                    if let HarnessEvent::ConfigOptions { options } = event {
                        *self.agent_config_options.lock() = options;
                        self.ctx.emit_event(HarnessEvent::ConfigOptions {
                            options: self.merged_config_options(),
                        });
                        continue;
                    }
                    self.ctx.emit_event(event);
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
        SessionUpdate::UserMessageChunk(_) => Vec::new(), // we already show the user input
        SessionUpdate::ToolCall(tc) => serde_json::to_value(&tc)
            .ok()
            .map(|w| tool_call_events(&w))
            .unwrap_or_default(),
        SessionUpdate::ToolCallUpdate(u) => serde_json::to_value(&u)
            .ok()
            .map(|w| tool_call_update_events(&w))
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
        // Not consumed by the PR1 UI (reserved for later PRs).
        SessionUpdate::AvailableCommandsUpdate(_)
        | SessionUpdate::CurrentModeUpdate(_)
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
    let raw_output = w.get("rawOutput").map(compact_json_string);
    match status {
        "completed" => vec![HarnessEvent::ToolCallEnd {
            tool_call_id: tool_call_id.to_string(),
            ok: true,
            content: content.or(raw_output),
            error: None,
            file_changes,
        }],
        "failed" => vec![HarnessEvent::ToolCallEnd {
            tool_call_id: tool_call_id.to_string(),
            ok: false,
            content: content.clone(),
            error: raw_output.or(Some("tool failed".to_string())),
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
    let raw_output = w.get("rawOutput").map(compact_json_string);
    match status {
        Some("completed") => vec![HarnessEvent::ToolCallEnd {
            tool_call_id: tool_call_id.to_string(),
            ok: true,
            content: content.or(raw_output),
            error: None,
            file_changes,
        }],
        Some("failed") => vec![HarnessEvent::ToolCallEnd {
            tool_call_id: tool_call_id.to_string(),
            ok: false,
            content: content.clone(),
            error: raw_output.or(Some("tool failed".to_string())),
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

/// Compute the CLI args injected for the chosen spawn-option values
/// (`{arg}={value}` for values marked `inject`; unknown ids are ignored).
fn spawn_option_args(specs: &[SpawnOptionSpec], values: &HashMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    for spec in specs {
        let Some(chosen) = values.get(&spec.option_id) else {
            continue;
        };
        if let Some(v) = spec.values.iter().find(|v| &v.id == chosen) {
            if v.inject {
                out.push(format!("{}={}", spec.arg, v.id));
            }
        }
    }
    out
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

    /// Spawn options (CLI-flag-backed): chosen injectable values become
    /// `{arg}={value}` args; the default value and unknown ids inject nothing;
    /// synthesis marks the category "spawn" with the chosen (or first) current.
    #[test]
    fn spawn_option_args_and_synthesis() {
        use crate::plugins::harness::registry::{SpawnOptionSpec, SpawnOptionValueSpec};
        let spec = SpawnOptionSpec {
            option_id: "approval-mode".to_string(),
            name: "Approval Mode".to_string(),
            arg: "--approval-mode".to_string(),
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
        let specs = vec![spec.clone()];

        // Default value (inject=false) → no args.
        let mut values = HashMap::from([("approval-mode".to_string(), "always-ask".to_string())]);
        assert!(spawn_option_args(&specs, &values).is_empty());
        // Injectable values → {arg}={id}.
        values.insert("approval-mode".to_string(), "yolo".to_string());
        assert_eq!(spawn_option_args(&specs, &values), ["--approval-mode=yolo"]);
        values.insert("approval-mode".to_string(), "write".to_string());
        assert_eq!(
            spawn_option_args(&specs, &values),
            ["--approval-mode=write"]
        );
        // Unknown value id → ignored.
        values.insert("approval-mode".to_string(), "bogus".to_string());
        assert!(spawn_option_args(&specs, &values).is_empty());

        // Synthesis: current = chosen value, category "spawn", full value list.
        let opt = synth_spawn_option(&spec, values.get("approval-mode"));
        assert_eq!(opt.option_id, "approval-mode");
        assert_eq!(opt.category.as_deref(), Some("spawn"));
        assert_eq!(opt.current.as_deref(), Some("bogus"));
        assert_eq!(opt.values.len(), 3);
        // Synthesis without a preference falls back to the first value.
        let opt = synth_spawn_option(&spec, None);
        assert_eq!(opt.current.as_deref(), Some("always-ask"));
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
}
