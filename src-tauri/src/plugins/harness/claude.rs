//! Claude Code native driver — the deep-integration adapter for Anthropic's
//! `claude` CLI (preferred over the ACP fallback when a real native binary is
//! installed).
//!
//! Transport: bidirectional stream-json over the child's stdio — exactly the
//! wire Anthropic's own SDKs drive. One long-lived process per session:
//! user prompts are JSONL user-message lines, turn ends arrive as a `result`
//! frame (not a request/response correlation), and `--permission-prompt-tool
//! stdio` routes every permission ask to us as a `can_use_tool` control frame.
//!
//! Wire references: code.claude.com/docs/en/headless + the Agent SDK source
//! (subprocess_cli.py / query.py). Everything unknown is debug-skipped — the
//! CLI adds frame types between versions and must never kill the session.

use crate::plugins::harness::adapter::{
    spawn_harness_child, DriverCtx, DriverHandle, DriverSpawn, HarnessAdapter, HarnessCommand,
};
use crate::plugins::harness::model::{
    HarnessApprovalOption, HarnessApprovalRequest, HarnessEvent, HarnessUsage,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::mpsc;

/// The Claude Code native adapter.
pub struct ClaudeAdapter;

#[async_trait::async_trait]
impl HarnessAdapter for ClaudeAdapter {
    async fn start(
        &self,
        spawn: DriverSpawn,
        cmd_rx: mpsc::UnboundedReceiver<HarnessCommand>,
    ) -> Result<DriverHandle, String> {
        ClaudeDriver::launch(spawn, cmd_rx).await
    }
}

/// Minimum plausible size of the real native binary. The npm package ships a
/// <4KB placeholder stub at `bin/claude.exe` that postinstall replaces with the
/// actual binary (verified against @anthropic-ai/claude-code@2.1.301's
/// install.cjs: stub check `size < 4096`, real binary ~500MB). Anything below
/// this threshold is a stub — a failed postinstall (--omit=optional, download
/// error) must not be mistaken for a working native claude.
const REAL_BINARY_MIN_BYTES: u64 = 1_000_000;

/// Whether a candidate path points to the REAL native binary (not the stub).
/// `fs::metadata` follows symlinks — on Unix the PATH-resolved `claude` is
/// typically a symlink into the npm package, and the guard reads the target.
fn is_real_native_binary(p: &std::path::Path) -> bool {
    match std::fs::metadata(p) {
        Ok(md) => md.is_file() && md.len() >= REAL_BINARY_MIN_BYTES,
        Err(_) => false,
    }
}

/// Resolve the real native exe inside an npm global install layout:
/// `<npm_prefix>/node_modules/@anthropic-ai/claude-code/bin/claude(.exe)` —
/// exactly what install.cjs places (hardlink of the platform-package binary)
/// and exactly what the PATH shim execs.
fn npm_package_binary(npm_prefix: &std::path::Path) -> Option<std::path::PathBuf> {
    let exe = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    let candidate = npm_prefix
        .join("node_modules")
        .join("@anthropic-ai")
        .join("claude-code")
        .join("bin")
        .join(exe);
    is_real_native_binary(&candidate).then_some(candidate)
}

/// Locate a REAL native `claude` binary — never an npm `.cmd`/`.ps1` shim:
/// shims get rewritten through `cmd.exe` (BatBadBut-class injection surface;
/// Anthropic's own Python SDK refuses to spawn them too).
///
/// Probe order:
/// 1. Native installer location (`~/.local/bin/claude[.exe]`) — Anthropic's
///    recommended install, which also auto-updates.
/// 2. PATH walk (real executables only; the size guard rejects npm's <4KB stub).
/// 3. npm global install: the PATH shim reveals the npm prefix; the real exe
///    sits inside the package (`node_modules/@anthropic-ai/claude-code/bin/`).
///    Both layouts probed: shim beside node_modules (Windows npm default) and
///    shim in `<prefix>/bin` with modules in `<prefix>/lib/node_modules` (Unix).
/// 4. Windows default npm prefix (`%APPDATA%\npm`) as a final fallback.
pub fn detect_native_binary() -> Option<std::path::PathBuf> {
    let exe_name = if cfg!(windows) {
        "claude.exe"
    } else {
        "claude"
    };
    // 1. Native installer.
    let home_var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    if let Some(home) = std::env::var_os(home_var) {
        let p = std::path::Path::new(&home)
            .join(".local")
            .join("bin")
            .join(exe_name);
        if is_real_native_binary(&p) {
            return Some(p);
        }
    }
    // 2. PATH walk: on Windows accept only a true .exe (the resolver also
    //    finds .cmd/.ps1 shims, which we reject by extension).
    if let Some(p) = crate::utils::exec_resolve::resolve_executable("claude") {
        let is_real_exe = if cfg!(windows) {
            p.extension()
                .map(|e| e.eq_ignore_ascii_case("exe"))
                .unwrap_or(false)
        } else {
            true
        };
        if is_real_exe && is_real_native_binary(&p) {
            return Some(p);
        }
    }
    // 3. npm global install via the shim's prefix.
    if let Some(shim) = crate::utils::exec_resolve::resolve_executable("claude") {
        if let Some(prefix) = shim.parent() {
            // Windows npm layout: <prefix>\claude.cmd beside <prefix>\node_modules.
            if let Some(p) = npm_package_binary(prefix) {
                return Some(p);
            }
            // Unix npm layout: <prefix>/bin/claude → <prefix>/lib/node_modules.
            if let Some(p) = npm_package_binary(&prefix.join("lib")) {
                return Some(p);
            }
        }
    }
    // 4. Windows default npm prefix (covers PATH orders that hide the shim).
    #[cfg(windows)]
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let prefix = std::path::Path::new(&appdata).join("npm");
        if let Some(p) = npm_package_binary(&prefix) {
            return Some(p);
        }
    }
    None
}

/// Shared state of one running Claude driver.
struct ClaudeDriver {
    ctx: DriverCtx,
    tool_label: String,
    stdin: Arc<tokio::sync::Mutex<ChildStdin>>,
    session_id: parking_lot::Mutex<Option<String>>,
    prompt_running: AtomicBool,
    /// Set when we sent an interrupt for the running turn; the terminal result
    /// frame then maps to `cancelled` instead of `error`.
    interrupt_requested: AtomicBool,
    /// Our control-request id counter (host→CLI frames).
    req_counter: AtomicU64,
    /// Approval correlation: our approval uuid → the CLI's control request id.
    permits: parking_lot::Mutex<HashMap<String, String>>,
    /// Current model (from init frame / set_model; reported in config options).
    current_model: parking_lot::Mutex<Option<String>>,
    /// Current effort level (spawn-time flag; report-only — changes apply on respawn).
    thinking_level: parking_lot::Mutex<Option<String>>,
}

impl ClaudeDriver {
    async fn launch(
        spawn: DriverSpawn,
        cmd_rx: mpsc::UnboundedReceiver<HarnessCommand>,
    ) -> Result<DriverHandle, String> {
        let tool_label = spawn.spec.name.to_string();
        let binary = detect_native_binary().ok_or_else(|| {
            format!(
                "[{tool_label}] 未找到原生 claude 可执行文件（.cmd/.ps1 shim 不受支持；\
                 请使用官方安装器安装原生版本）"
            )
        })?;
        let mut args: Vec<String> = vec![
            "--output-format=stream-json".into(),
            "--verbose".into(),
            "--input-format=stream-json".into(),
            // Route every permission ask to the host over the control channel —
            // exactly what Anthropic's SDK passes for canUseTool.
            "--permission-prompt-tool=stdio".into(),
            // Surface subagent text as frames tagged with parent_tool_use_id.
            "--forward-subagent-text".into(),
            "--setting-sources=user,project".into(),
        ];
        if let Some(mode) = &spawn.permission_mode {
            args.push(format!("--permission-mode={mode}"));
        }
        // Model & effort preferences (frontend-stored, applied at spawn).
        if let Some(model) = &spawn.initial_model {
            args.push(format!("--model={model}"));
        }
        if let Some(effort) = &spawn.initial_thinking {
            args.push(format!("--effort={effort}"));
        }
        if let Some(sid) = &spawn.harness_session_id {
            args.push(format!("--resume={sid}"));
        }
        let program = binary.to_string_lossy().into_owned();
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
        let driver = Arc::new(ClaudeDriver {
            ctx: ctx.clone(),
            tool_label: tool_label.clone(),
            stdin: Arc::new(tokio::sync::Mutex::new(stdin)),
            session_id: parking_lot::Mutex::new(None),
            prompt_running: AtomicBool::new(false),
            interrupt_requested: AtomicBool::new(false),
            req_counter: AtomicU64::new(1),
            permits: parking_lot::Mutex::new(HashMap::new()),
            current_model: parking_lot::Mutex::new(spawn.initial_model.clone()),
            thinking_level: parking_lot::Mutex::new(spawn.initial_thinking.clone()),
        });

        if let Some(stderr) = stderr {
            let label = tool_label.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!("[harness:{label}] stderr: {line}");
                }
            });
        }
        {
            let d = driver.clone();
            tokio::spawn(async move {
                d.read_loop(stdout).await;
            });
        }

        // Handshake: the SDK-style initialize control frame (registers nothing;
        // its response carries pending_permission_requests on reconnect).
        ctx.emit_state("connecting", None);
        let init_id = format!(
            "aio_init_{}",
            driver.req_counter.fetch_add(1, Ordering::SeqCst)
        );
        driver
            .send_control_request(&init_id, json!({ "subtype": "initialize" }))
            .await
            .map_err(|e| format!("[{tool_label}] 控制通道初始化失败: {e}"))?;
        // No response correlation is needed to proceed — stream-json sessions
        // accept prompts as soon as stdin is live; system/init will follow.
        ctx.emit_state("idle", None);
        // Synthesized config options: the model selector (live via set_model)
        // and effort/thinking selector (spawn-time --effort flag; the current
        // value here reports the spawn-time setting until a respawn applies a
        // new one). The system/init frame's model field corrects the current
        // model once it arrives.
        ctx.emit_event(HarnessEvent::ConfigOptions {
            options: claude_config_options(
                driver.current_model.lock().clone(),
                driver.thinking_level.lock().clone(),
            ),
        });

        {
            let d = driver.clone();
            tokio::spawn(async move {
                d.command_loop(cmd_rx).await;
            });
        }

        Ok(DriverHandle { child: Some(child) })
    }

    async fn write_line(&self, line: &str) -> Result<(), String> {
        let mut w = self.stdin.lock().await;
        w.write_all(line.as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        w.write_all(b"\n").await.map_err(|e| e.to_string())
    }

    /// Host→CLI control request (initialize / set_permission_mode / interrupt).
    async fn send_control_request(&self, request_id: &str, request: Value) -> Result<(), String> {
        let line = json!({
            "type": "control_request",
            "request_id": request_id,
            "request": request
        });
        self.write_line(&line.to_string()).await
    }

    /// Read loop: NDJSON frames until the transport dies.
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
            if let Ok(v) = serde_json::from_str::<Value>(&line) {
                self.handle_frame(&v).await;
            } else {
                tracing::debug!("[harness:{}] 非 JSON 行: {line}", self.tool_label);
            }
        }
        // Transport closed / cancelled: end the session for everyone. A turn
        // still in flight surfaces as a terminal error event so the UI resets.
        self.ctx.token.cancel();
        if self.prompt_running.swap(false, Ordering::SeqCst) {
            self.ctx.emit_event(HarnessEvent::TurnEnded {
                stop_reason: "error".to_string(),
                error: Some("claude 进程已退出".to_string()),
            });
            self.ctx.emit_state("idle", None);
        }
    }

    /// Dispatch one stream-json frame by its documented top-level `type`.
    async fn handle_frame(self: &Arc<Self>, v: &Value) {
        match v.get("type").and_then(Value::as_str) {
            Some("system") => self.handle_system(v),
            Some("assistant") => {
                for event in assistant_frame_events(v) {
                    self.ctx.emit_event(event);
                }
            }
            Some("user") => {
                for event in user_frame_events(v) {
                    self.ctx.emit_event(event);
                }
            }
            Some("result") => self.handle_result(v).await,
            Some("control_request") => {
                let d = self.clone();
                let frame = v.clone();
                tauri::async_runtime::spawn(async move {
                    d.handle_control_request(&frame).await;
                });
            }
            Some("control_response") => {
                // Responses to our own initialize/mode/interrupt frames — fire
                // and forget by design.
                tracing::debug!(
                    "[harness:{}] control_response: {}",
                    self.tool_label,
                    v.pointer("/response/subtype")
                        .and_then(|s| s.as_str())
                        .unwrap_or("?")
                );
            }
            Some(other) => {
                // stream_event / rate_limit_event / future types: skipped, never
                // fatal (the CLI adds frame types between versions).
                tracing::debug!("[harness:{}] 跳过帧类型 {other}", self.tool_label);
            }
            None => tracing::debug!("[harness:{}] 无 type 字段的帧", self.tool_label),
        }
    }

    /// system/init → session/model; compact_boundary → notice.
    fn handle_system(&self, v: &Value) {
        match v.get("subtype").and_then(Value::as_str) {
            Some("init") => {
                let sid = v
                    .get("session_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let model = v.get("model").and_then(Value::as_str).map(str::to_string);
                if let Some(sid) = &sid {
                    *self.session_id.lock() = Some(sid.clone());
                    self.ctx.set_harness_session_id(sid);
                }
                // The init frame reports the CLI's actual model — update our
                // config-option state (the spawn preference may be an alias
                // like "sonnet" while init reports the resolved full id).
                if let Some(m) = &model {
                    *self.current_model.lock() = Some(m.clone());
                }
                self.ctx.emit_event(HarnessEvent::SessionInfo {
                    harness_session_id: sid,
                    model,
                });
                self.ctx.emit_event(HarnessEvent::ConfigOptions {
                    options: claude_config_options(
                        self.current_model.lock().clone(),
                        self.thinking_level.lock().clone(),
                    ),
                });
            }
            Some("compact_boundary") => {
                self.ctx.emit_event(HarnessEvent::Notice {
                    message: "上下文已压缩（compact boundary）".to_string(),
                });
            }
            _ => {}
        }
    }

    /// Terminal `result` frame: usage/cost + TurnEnded + idle.
    async fn handle_result(&self, v: &Value) {
        if !self.prompt_running.swap(false, Ordering::SeqCst) {
            return; // stray result outside a turn (e.g. resume replay)
        }
        self.interrupt_requested.store(false, Ordering::SeqCst);
        // session_id on the result covers CLI versions that emit it before init.
        if let Some(sid) = v.get("session_id").and_then(Value::as_str) {
            if self.session_id.lock().is_none() {
                *self.session_id.lock() = Some(sid.to_string());
                self.ctx.set_harness_session_id(sid);
            }
        }
        let usage = HarnessUsage {
            input_tokens: v.pointer("/usage/input_tokens").and_then(Value::as_u64),
            output_tokens: v.pointer("/usage/output_tokens").and_then(Value::as_u64),
            context_tokens: v.pointer("/usage/input_tokens").and_then(Value::as_u64),
            context_window: None,
            cost_usd: v.get("total_cost_usd").and_then(Value::as_f64),
        };
        self.ctx.emit_event(HarnessEvent::UsageUpdate { usage });
        let is_error = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
        let subtype = v
            .get("subtype")
            .and_then(Value::as_str)
            .unwrap_or("success");
        let (stop_reason, error) = if self.interrupt_requested.load(Ordering::SeqCst) {
            ("cancelled".to_string(), None)
        } else if is_error {
            let msg = v
                .get("result")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    v.get("errors")
                        .and_then(Value::as_array)
                        .and_then(|a| a.first())
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
            ("error".to_string(), msg)
        } else {
            (subtype.to_string(), None)
        };
        self.ctx
            .emit_event(HarnessEvent::TurnEnded { stop_reason, error });
        self.ctx.emit_state("idle", None);
    }

    /// CLI→host control request: can_use_tool → approval round-trip; anything
    /// else gets an error control_response so the CLI never blocks on us.
    async fn handle_control_request(self: &Arc<Self>, v: &Value) {
        let Some(cli_req_id) = v.get("request_id").and_then(Value::as_str) else {
            return;
        };
        let subtype = v
            .pointer("/request/subtype")
            .and_then(Value::as_str)
            .unwrap_or("");
        if subtype != "can_use_tool" {
            let reply = json!({
                "type": "control_response",
                "response": {
                    "subtype": "error",
                    "request_id": cli_req_id,
                    "error": format!("unsupported control request: {subtype}")
                }
            });
            let _ = self.write_line(&reply.to_string()).await;
            return;
        }
        let input = v.pointer("/request/input").cloned().unwrap_or(Value::Null);
        let request = HarnessApprovalRequest {
            request_id: uuid::Uuid::new_v4().to_string(),
            kind: "tool".to_string(),
            title: v
                .pointer("/request/title")
                .and_then(Value::as_str)
                .map(str::to_string),
            tool_name: v
                .pointer("/request/tool_name")
                .and_then(Value::as_str)
                .map(str::to_string),
            arguments: Some(input.clone()),
            command: v
                .pointer("/request/input/command")
                .and_then(Value::as_str)
                .map(str::to_string),
            file_path: None,
            preview_diff: None,
            options: vec![
                HarnessApprovalOption {
                    option_id: "allow".to_string(),
                    name: "允许".to_string(),
                    kind: "allow_once".to_string(),
                },
                HarnessApprovalOption {
                    option_id: "deny".to_string(),
                    name: "拒绝".to_string(),
                    kind: "reject_once".to_string(),
                },
            ],
        };
        self.permits
            .lock()
            .insert(request.request_id.clone(), cli_req_id.to_string());
        let our_request_id = request.request_id.clone();
        let decision = self.ctx.request_approval(request).await;
        self.permits.lock().remove(&our_request_id);
        let response = match decision {
            Some(d) if !d.cancelled && d.option_id.as_deref() == Some("allow") => json!({
                "behavior": "allow",
                // Echo back the tool input verbatim (required since CLI < v2.1.207).
                "updatedInput": input
            }),
            _ => json!({
                "behavior": "deny",
                "message": "用户拒绝了该操作"
            }),
        };
        let reply = json!({
            "type": "control_response",
            "response": {
                "subtype": "success",
                "request_id": cli_req_id,
                "response": response
            }
        });
        let _ = self.write_line(&reply.to_string()).await;
    }

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
                HarnessCommand::Prompt { text } => {
                    if self.prompt_running.load(Ordering::SeqCst) {
                        self.ctx.emit_event(HarnessEvent::Error {
                            message: "上一个回合仍在运行，请先停止或等待其结束".to_string(),
                        });
                        continue;
                    }
                    let line = json!({
                        "type": "user",
                        "session_id": "",
                        "message": {
                            "role": "user",
                            "content": [ { "type": "text", "text": text } ]
                        },
                        "parent_tool_use_id": null
                    });
                    if self.write_line(&line.to_string()).await.is_err() {
                        self.ctx.emit_event(HarnessEvent::Error {
                            message: "发送失败：claude 进程已退出".to_string(),
                        });
                        continue;
                    }
                    self.prompt_running.store(true, Ordering::SeqCst);
                    self.interrupt_requested.store(false, Ordering::SeqCst);
                    self.ctx.emit_event(HarnessEvent::TurnStarted);
                    self.ctx.emit_state("running", None);
                }
                HarnessCommand::CancelTurn => {
                    if !self.prompt_running.load(Ordering::SeqCst) {
                        continue;
                    }
                    self.interrupt_requested.store(true, Ordering::SeqCst);
                    let id = format!(
                        "aio_int_{}",
                        self.req_counter.fetch_add(1, Ordering::SeqCst)
                    );
                    if let Err(e) = self
                        .send_control_request(&id, json!({ "subtype": "interrupt" }))
                        .await
                    {
                        tracing::debug!("[harness:{}] interrupt 发送失败: {e}", self.tool_label);
                    }
                }
                HarnessCommand::SetPermissionMode { mode } => {
                    let id = format!(
                        "aio_mode_{}",
                        self.req_counter.fetch_add(1, Ordering::SeqCst)
                    );
                    if let Err(e) = self
                        .send_control_request(
                            &id,
                            json!({ "subtype": "set_permission_mode", "mode": mode }),
                        )
                        .await
                    {
                        tracing::debug!(
                            "[harness:{}] set_permission_mode 发送失败: {e}",
                            self.tool_label
                        );
                    }
                }
                HarnessCommand::SetConfigOption { config_id, value } => {
                    self.handle_set_config_option(config_id, value).await;
                }
            }
        }
    }

    /// Set a config option: "model" → live set_model control frame;
    /// "effort" → report-only (the --effort spawn flag applies on respawn;
    /// the frontend persists the preference and stops the session to apply).
    async fn handle_set_config_option(&self, config_id: String, value: Value) {
        let Some(val) = value.as_str() else {
            return;
        };
        match config_id.as_str() {
            "model" => {
                let id = format!(
                    "aio_model_{}",
                    self.req_counter.fetch_add(1, Ordering::SeqCst)
                );
                if let Err(e) = self
                    .send_control_request(&id, json!({ "subtype": "set_model", "model": val }))
                    .await
                {
                    tracing::debug!("[harness:{}] set_model 发送失败: {e}", self.tool_label);
                    return;
                }
                // Optimistic update: the set_model control response confirms
                // (fire-and-forget); the next assistant frame's message.model
                // field reflects the CLI's actual model.
                *self.current_model.lock() = Some(val.to_string());
                self.ctx.emit_event(HarnessEvent::ConfigOptions {
                    options: claude_config_options(
                        self.current_model.lock().clone(),
                        self.thinking_level.lock().clone(),
                    ),
                });
            }
            "effort" => {
                // Report-only: the actual --effort flag applies at the next
                // spawn (the frontend persists the preference and stops the
                // session runtime so the next prompt respawns with it).
                *self.thinking_level.lock() = Some(val.to_string());
                self.ctx.emit_event(HarnessEvent::ConfigOptions {
                    options: claude_config_options(
                        self.current_model.lock().clone(),
                        self.thinking_level.lock().clone(),
                    ),
                });
            }
            _ => {
                tracing::debug!(
                    "[harness:{}] 忽略未知 config 选项 {config_id}",
                    self.tool_label
                );
            }
        }
    }
}

// ===== frame → event mapping (pure, unit-tested) =====

/// Synthesize the config-option set for the Claude native driver: a model
/// selector (stable aliases + the CLI's resolved current model) and an
/// effort/thinking selector (spawn-time --effort flag values). Pure over the
/// current values so the `harness_tool_config_options` command can reuse it
/// without a live driver (first-use dropdown data).
pub fn claude_config_options(
    model: Option<String>,
    effort: Option<String>,
) -> Vec<crate::plugins::harness::model::HarnessConfigOption> {
    use crate::plugins::harness::model::{HarnessConfigOption, HarnessConfigValue};
    let model_values = vec![
        HarnessConfigValue {
            id: "sonnet".into(),
            name: Some("Sonnet".into()),
        },
        HarnessConfigValue {
            id: "opus".into(),
            name: Some("Opus".into()),
        },
        HarnessConfigValue {
            id: "haiku".into(),
            name: Some("Haiku".into()),
        },
    ];
    let effort_values = vec![
        HarnessConfigValue {
            id: "low".into(),
            name: None,
        },
        HarnessConfigValue {
            id: "medium".into(),
            name: None,
        },
        HarnessConfigValue {
            id: "high".into(),
            name: None,
        },
    ];
    vec![
        HarnessConfigOption {
            option_id: "model".into(),
            name: Some("Model".into()),
            kind: "select".into(),
            category: Some("model".into()),
            current: model,
            values: model_values,
            boolean_value: None,
        },
        HarnessConfigOption {
            option_id: "effort".into(),
            name: Some("Effort".into()),
            kind: "select".into(),
            category: Some("thought_level".into()),
            current: effort,
            values: effort_values,
            boolean_value: None,
        },
    ]
}

/// tool_use content block → ToolCallStart.
fn tool_use_start(block: &Value) -> Option<HarnessEvent> {
    Some(HarnessEvent::ToolCallStart {
        tool_call_id: block.get("id")?.as_str()?.to_string(),
        name: block
            .get("name")
            .and_then(Value::as_str)
            .map(str::to_string),
        title: None,
        tool_kind: None,
        arguments: block.get("input").map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        }),
    })
}

/// Assistant frame: main-thread text/thinking → deltas, tool_use → call starts;
/// subagent-attributed frames (parent_tool_use_id set) → SubagentDelta.
fn assistant_frame_events(v: &Value) -> Vec<HarnessEvent> {
    let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    let is_subagent = v
        .get("parent_tool_use_id")
        .and_then(Value::as_str)
        .is_some();
    let agent_name = v
        .get("agent_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut out = Vec::new();
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                let text = block.get("text").and_then(Value::as_str).unwrap_or("");
                if text.is_empty() {
                    continue;
                }
                if is_subagent {
                    out.push(HarnessEvent::SubagentDelta {
                        name: agent_name.clone(),
                        text: text.to_string(),
                    });
                } else {
                    out.push(HarnessEvent::TextDelta {
                        text: text.to_string(),
                    });
                }
            }
            Some("thinking") if !is_subagent => {
                let text = block.get("thinking").and_then(Value::as_str).unwrap_or("");
                if !text.is_empty() {
                    out.push(HarnessEvent::ThinkingDelta {
                        text: text.to_string(),
                    });
                }
            }
            Some("tool_use") if !is_subagent => {
                if let Some(e) = tool_use_start(block) {
                    out.push(e);
                }
            }
            _ => {}
        }
    }
    out
}

/// tool_result content block (string or block array) → plain text.
fn tool_result_text(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(items) => {
            let mut buf = String::new();
            for item in items {
                if let Some(t) = item.pointer("/text") {
                    if let Some(s) = t.as_str() {
                        buf.push_str(s);
                    }
                }
            }
            (!buf.is_empty()).then_some(buf)
        }
        _ => None,
    }
}

/// User frame: tool_result blocks → ToolCallEnd (matched by tool_use_id).
fn user_frame_events(v: &Value) -> Vec<HarnessEvent> {
    let Some(blocks) = v.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for block in blocks {
        if block.get("type").and_then(Value::as_str) != Some("tool_result") {
            continue;
        }
        let Some(tool_call_id) = block.get("tool_use_id").and_then(Value::as_str) else {
            continue;
        };
        let is_error = block
            .get("is_error")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let text = tool_result_text(block.get("content").unwrap_or(&Value::Null));
        out.push(HarnessEvent::ToolCallEnd {
            tool_call_id: tool_call_id.to_string(),
            ok: !is_error,
            content: if is_error { None } else { text.clone() },
            error: if is_error { text } else { None },
            file_changes: Vec::new(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Literal assistant frames from the documented stream-json schema.
    #[test]
    fn maps_assistant_text_thinking_and_tool_use() {
        let frame = json!({
            "type": "assistant",
            "session_id": "s1",
            "parent_tool_use_id": null,
            "message": {
                "role": "assistant",
                "content": [
                    { "type": "thinking", "thinking": "let me look" },
                    { "type": "text", "text": "I'll inspect the repo." },
                    { "type": "tool_use", "id": "toolu_01", "name": "Bash", "input": { "command": "ls" } }
                ]
            }
        });
        let events = assistant_frame_events(&frame);
        assert!(matches!(
            &events[..],
            [
                HarnessEvent::ThinkingDelta { text },
                HarnessEvent::TextDelta { text: t2 },
                HarnessEvent::ToolCallStart { tool_call_id, name, .. }
            ] if text == "let me look" && t2 == "I'll inspect the repo." && tool_call_id == "toolu_01" && name.as_deref() == Some("Bash")
        ));
    }

    /// Subagent-attributed frames stream into SubagentDelta, never main text.
    #[test]
    fn maps_subagent_frames_to_subagent_delta() {
        let frame = json!({
            "type": "assistant",
            "parent_tool_use_id": "toolu_parent",
            "agent_id": "agent_x",
            "message": { "content": [ { "type": "text", "text": "working on it" } ] }
        });
        match assistant_frame_events(&frame).as_slice() {
            [HarnessEvent::SubagentDelta { name, text }] => {
                assert_eq!(text, "working on it");
                assert_eq!(name.as_deref(), Some("agent_x"));
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    /// tool_result frames close the matching tool call; is_error flips ok/error.
    #[test]
    fn maps_tool_result_and_error_flag() {
        let ok_frame = json!({
            "type": "user",
            "message": { "content": [
                { "type": "tool_result", "tool_use_id": "t1", "content": "done" }
            ] }
        });
        match user_frame_events(&ok_frame).as_slice() {
            [HarnessEvent::ToolCallEnd {
                tool_call_id,
                ok,
                content,
                error,
                ..
            }] => {
                assert_eq!(tool_call_id, "t1");
                assert!(ok);
                assert_eq!(content.as_deref(), Some("done"));
                assert!(error.is_none());
            }
            other => panic!("unexpected events: {other:?}"),
        }
        let err_frame = json!({
            "type": "user",
            "message": { "content": [
                { "type": "tool_result", "tool_use_id": "t1", "content": [ { "type": "text", "text": "boom" } ], "is_error": true }
            ] }
        });
        match user_frame_events(&err_frame).as_slice() {
            [HarnessEvent::ToolCallEnd { ok, error, .. }] => {
                assert!(!ok);
                assert_eq!(error.as_deref(), Some("boom"));
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    /// Array-form tool_result content concatenates its text blocks.
    #[test]
    fn tool_result_array_content_concatenates() {
        let content = json!([ { "type": "text", "text": "a" }, { "type": "text", "text": "b" } ]);
        assert_eq!(tool_result_text(&content).as_deref(), Some("ab"));
        assert_eq!(tool_result_text(&json!("plain")).as_deref(), Some("plain"));
        assert_eq!(tool_result_text(&json!({})), None);
    }

    /// Temp prefix laid out like an npm global install.
    fn npm_prefix(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aio-claude-npm-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("node_modules/@anthropic-ai/claude-code/bin")).unwrap();
        d
    }

    /// The npm package's `bin/claude.exe` slot: a real (≥1MB) binary resolves;
    /// the <4KB placeholder stub a failed postinstall leaves behind does not.
    #[test]
    fn npm_package_binary_accepts_real_binary_rejects_stub() {
        let prefix = npm_prefix("real");
        let exe = prefix
            .join("node_modules/@anthropic-ai/claude-code/bin")
            .join(if cfg!(windows) {
                "claude.exe"
            } else {
                "claude"
            });
        // Real binary: over the 1MB threshold.
        std::fs::write(&exe, vec![0u8; REAL_BINARY_MIN_BYTES as usize + 1]).unwrap();
        assert_eq!(npm_package_binary(&prefix), Some(exe.clone()));

        // Stub: the shipped placeholder is <4KB — must be rejected.
        std::fs::write(&exe, vec![0u8; 4095]).unwrap();
        assert_eq!(npm_package_binary(&prefix), None);

        // Missing slot → None.
        std::fs::remove_file(&exe).unwrap();
        assert_eq!(npm_package_binary(&prefix), None);
        let _ = std::fs::remove_dir_all(&prefix);
    }

    /// The size guard is the only stub/real discriminator — a 1MB boundary file
    /// at exactly the threshold is accepted (>= comparison).
    #[test]
    fn real_binary_threshold_is_inclusive() {
        let prefix = npm_prefix("edge");
        let exe = prefix
            .join("node_modules/@anthropic-ai/claude-code/bin")
            .join(if cfg!(windows) {
                "claude.exe"
            } else {
                "claude"
            });
        std::fs::write(&exe, vec![0u8; REAL_BINARY_MIN_BYTES as usize]).unwrap();
        assert_eq!(npm_package_binary(&prefix), Some(exe.clone()));
        std::fs::write(&exe, vec![0u8; REAL_BINARY_MIN_BYTES as usize - 1]).unwrap();
        assert_eq!(npm_package_binary(&prefix), None);
        let _ = std::fs::remove_dir_all(&prefix);
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::plugins::harness::adapter::{DriverCtx, DriverSpawn, HarnessEmitter};
    use crate::plugins::harness::model::DriverEvent;
    use crate::plugins::harness::registry;
    use crate::plugins::harness::store;
    use tokio::sync::mpsc;

    struct TestEmitter {
        conn: parking_lot::Mutex<rusqlite::Connection>,
    }

    impl HarnessEmitter for TestEmitter {
        fn emit_approval_requested(
            &self,
            _t: &str,
            _s: &str,
            _r: crate::plugins::harness::model::HarnessApprovalRequest,
        ) {
        }
        fn emit_approval_expired(&self, _r: &str) {}
        fn set_harness_session_id(&self, row: &str, sid: &str) {
            store::set_session_harness_id(&self.conn.lock(), row, sid).expect("persist sid");
        }
    }

    /// Live roundtrip against a locally installed native claude binary.
    /// Ignored by default — needs `claude` (native install, logged in).
    /// `cargo test --manifest-path src-tauri/Cargo.toml claude_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "live: requires a native claude install on PATH with a logged-in session"]
    async fn claude_live_roundtrip() {
        if detect_native_binary().is_none() {
            eprintln!("[live] 原生 claude 未安装，跳过真实联调");
            return;
        }
        let ws = std::env::temp_dir().join(format!("aio-claude-live-{}", std::process::id()));
        std::fs::create_dir_all(&ws).unwrap();
        let workspace_path = store::canonical_workspace_path(&ws).unwrap();

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::core::db::create_base_schema(&conn).unwrap();
        crate::core::migrate::run_migrations(&conn, crate::core::db::CHAT_DB_MIGRATIONS).unwrap();
        const ROW_ID: &str = "claude-live-row";
        conn.execute(
            "INSERT INTO harness_sessions (id, tool_id, workspace_path, title, created_at, updated_at)
             VALUES (?1, 'claude', ?2, 'live test', 'now', 'now')",
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
            key: ("claude".to_string(), ROW_ID.to_string()),
            token: token.clone(),
            events: event_tx,
            approvals: Arc::new(dashmap::DashMap::new()),
        };

        let spec = registry::lookup("claude").unwrap();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<HarnessCommand>();
        let handle = ClaudeDriver::launch(
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
        .expect("claude 原生握手");

        cmd_tx
            .send(HarnessCommand::Prompt {
                text: "Reply with exactly the single word: hello".to_string(),
            })
            .unwrap();
        let mut got_text = false;
        let mut turn_error: Option<String> = None;
        let ended = tokio::time::timeout(std::time::Duration::from_secs(240), async {
            loop {
                match event_rx.recv().await.expect("driver events") {
                    DriverEvent::State { state, .. } => eprintln!("[live] state: {state}"),
                    DriverEvent::Event(HarnessEvent::TextDelta { text }) => {
                        eprintln!("[live] text: {text}");
                        if !text.trim().is_empty() {
                            got_text = true;
                        }
                    }
                    DriverEvent::Event(HarnessEvent::ThinkingDelta { text }) => {
                        eprintln!("[live] think: {text}");
                    }
                    DriverEvent::Event(HarnessEvent::SubagentDelta { name, text }) => {
                        eprintln!("[live] subagent {:?}: {text}", name);
                    }
                    DriverEvent::Event(HarnessEvent::Error { message }) => {
                        turn_error = Some(message)
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

        token.cancel();
        if let Some(child) = &handle.child {
            crate::plugins::harness::adapter::kill_child_tree(child).await;
        }
        let _ = std::fs::remove_dir_all(&ws);

        assert!(ended.is_ok(), "240s 内未收到 TurnEnded");
        assert!(turn_error.is_none(), "驱动器报错: {}", turn_error.unwrap());
        assert!(got_text, "回合内未收到任何文本流");
    }
}
