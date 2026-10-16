//! Harness adapter layer: the driver context every adapter gets, the command
//! channel into a running driver, and the shared child-process spawn helpers.
//!
//! Concurrency model: one long-lived **driver** per harness session owns the
//! child process. The manager sends [`HarnessCommand`]s over an unbounded mpsc
//! channel; the driver converts them to its native protocol (ACP JSON-RPC lines,
//! Claude control frames, ...). Everything the driver pushes back (normalized
//! events, state transitions, approval asks) flows through [`DriverCtx`].

use crate::core::events as ev;
use crate::core::state::DbState;
use crate::plugins::harness::model::{
    ApprovalDecision, DriverEvent, HarnessApprovalExpiredPayload, HarnessApprovalRequest,
    HarnessApprovalRequestedPayload, HarnessSessionKey,
};
use crate::plugins::harness::registry::{AcpCommand, HarnessToolKind, HarnessToolSpec};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// Commands the manager feeds into a running driver.
#[derive(Debug)]
pub enum HarnessCommand {
    /// Submit a user prompt (the driver decides the native transport form).
    Prompt { text: String },
    /// Request graceful cancellation of the current turn (protocol-level).
    CancelTurn,
    /// Switch the harness agent's permission mode (Claude Code native:
    /// set_permission_mode control frame; other adapters ignore it).
    SetPermissionMode { mode: String },
}

/// What a driver hands back on start: the child handle the manager needs for
/// hard-stop escalation. The command channel's sender half stays with the
/// manager (it constructs the channel and passes the receiver into `start`).
/// `child` is None for exec-per-turn adapters (Codex `exec --json`): the driver
/// tracks its current-turn process internally and reaps it on cancel; the
/// manager's token cancel plus the driver's own cleanup cover the stop path.
pub struct DriverHandle {
    pub child: Option<Arc<tokio::sync::Mutex<Child>>>,
}

/// Everything a driver task may do: emit normalized events / state, persist the
/// harness-side session id, and block on user approval decisions.
///
/// Frontend effects are abstracted behind [`HarnessEmitter`] so the driver is
/// unit-testable without a Tauri runtime (the production implementation wraps
/// an `AppHandle`; tests use an in-memory one).
#[derive(Clone)]
pub struct DriverCtx {
    pub emitter: Arc<dyn HarnessEmitter>,
    /// AIO client version (clientInfo in the ACP initialize handshake).
    pub client_version: String,
    pub key: HarnessSessionKey,
    pub token: CancellationToken,
    /// Normalized events + state transitions (consumed by the emitter task)
    pub events: mpsc::UnboundedSender<DriverEvent>,
    /// Pending approvals keyed by request id; `harness_respond_approval` resolves
    /// the oneshot (same pattern as the chat pipeline's PendingApprovals).
    pub approvals: Arc<dashmap::DashMap<String, oneshot::Sender<ApprovalDecision>>>,
}

/// Frontend/persistence effects a driver needs, abstracted for testability.
pub trait HarnessEmitter: Send + Sync {
    /// Emit `harness-approval-requested` to the frontend.
    fn emit_approval_requested(
        &self,
        tool_id: &str,
        session_row_id: &str,
        request: HarnessApprovalRequest,
    );
    /// Emit `harness-approval-expired` (the bubble removes itself).
    fn emit_approval_expired(&self, request_id: &str);
    /// Persist the harness-side session id for resume across restarts.
    fn set_harness_session_id(&self, session_row_id: &str, harness_session_id: &str);
}

/// Production emitter: Tauri events + the chat-history SQLite connection.
pub struct TauriHarnessEmitter {
    pub app: AppHandle,
}

impl HarnessEmitter for TauriHarnessEmitter {
    fn emit_approval_requested(
        &self,
        tool_id: &str,
        session_row_id: &str,
        request: HarnessApprovalRequest,
    ) {
        let _ = self.app.emit(
            ev::HARNESS_APPROVAL_REQUESTED,
            HarnessApprovalRequestedPayload {
                tool_id: tool_id.to_string(),
                session_row_id: session_row_id.to_string(),
                request,
            },
        );
    }

    fn emit_approval_expired(&self, request_id: &str) {
        let _ = self.app.emit(
            ev::HARNESS_APPROVAL_EXPIRED,
            HarnessApprovalExpiredPayload {
                request_id: request_id.to_string(),
            },
        );
    }

    fn set_harness_session_id(&self, session_row_id: &str, harness_session_id: &str) {
        // Brief lock scope; the chat pipeline uses the same direct-lock pattern.
        let state = self.app.state::<DbState>();
        let conn = state.0.lock();
        if let Err(e) = crate::plugins::harness::store::set_session_harness_id(
            &conn,
            session_row_id,
            harness_session_id,
        ) {
            tracing::warn!("[harness] 保存 harness 会话 ID 失败: {e}");
        }
    }
}

impl DriverCtx {
    /// Push a normalized harness event to the frontend.
    pub fn emit_event(&self, event: crate::plugins::harness::model::HarnessEvent) {
        let _ = self.events.send(DriverEvent::Event(event));
    }

    /// Push a run-state transition to the frontend.
    pub fn emit_state(&self, state: &str, error: Option<String>) {
        let _ = self.events.send(DriverEvent::State {
            state: state.to_string(),
            error,
        });
    }

    /// Ask the user for approval: emits `harness-approval-requested`, registers the
    /// oneshot, then blocks until the user answers. Returns `None` when the session
    /// died (token cancelled / channel dropped) — the caller then answers the
    /// harness with its native "cancelled" outcome and the bubble is expired.
    pub async fn request_approval(
        &self,
        request: HarnessApprovalRequest,
    ) -> Option<ApprovalDecision> {
        let request_id = request.request_id.clone();
        let (tx, rx) = oneshot::channel();
        self.approvals.insert(request_id.clone(), tx);
        self.emitter
            .emit_approval_requested(&self.key.0, &self.key.1, request);
        let decision = tokio::select! {
            _ = self.token.cancelled() => None,
            res = rx => res.ok(),
        };
        if decision.is_none() {
            self.approvals.remove(&request_id);
            self.emitter.emit_approval_expired(&request_id);
        }
        decision
    }

    /// Persist the harness-side session id (called by the driver once the native
    /// handshake reports it — needed for resume after app restarts).
    pub fn set_harness_session_id(&self, harness_session_id: &str) {
        self.emitter
            .set_harness_session_id(&self.key.1, harness_session_id);
    }
}

/// Adapter spawn input: the context, the resolved tool spec, and session facts.
pub struct DriverSpawn {
    pub ctx: DriverCtx,
    pub spec: HarnessToolSpec,
    pub workspace_path: String,
    pub harness_session_id: Option<String>,
    /// Initial permission mode (Claude Code native: --permission-mode spawn flag;
    /// other adapters ignore it).
    pub permission_mode: Option<String>,
}

/// One CLI harness adapter. Implementations own the whole native protocol for
/// their harness family (ACP, Claude Code stream-json, Codex exec --json, ...).
/// Dispatch happens on the registry's `HarnessToolKind`, so the trait itself
/// carries no id method.
#[async_trait::async_trait]
pub trait HarnessAdapter: Send + Sync {
    /// Spawn the driver for one harness session. Returns once the native session
    /// is established (handshake done) or failed — the driver keeps running in
    /// the background afterwards, consuming `cmd_rx`.
    async fn start(
        &self,
        spawn: DriverSpawn,
        cmd_rx: mpsc::UnboundedReceiver<HarnessCommand>,
    ) -> Result<DriverHandle, String>;
}

// ===== Shared child-process plumbing =====

/// Env vars passed through to harness child processes (SEC threat model parity
/// with the MCP stdio transport: the full host environment is NOT inherited —
/// only the whitelist below, so AIO-held secrets never leak into third-party CLIs).
fn inherited_env_whitelist() -> HashMap<String, String> {
    const KEYS: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "USERNAME",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "TMPDIR",
        "TMP",
        "TEMP",
        "APPDATA",
        "LOCALAPPDATA",
        "PROGRAMDATA",
        "SYSTEMROOT",
        "SYSTEMDRIVE",
        "PROGRAMFILES",
        "COMMONPROGRAMFILES",
        "PATHEXT",
        "ComSpec",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "NO_PROXY",
    ];
    let mut out = HashMap::new();
    for k in KEYS {
        if let Some(v) = std::env::var_os(k) {
            out.insert(k.to_string(), v.to_string_lossy().into_owned());
        }
    }
    out
}

/// Spawn a harness child: PATH-only executable resolution (SEC-06 — the workspace
/// may be model-writable, so the current directory must never be consulted),
/// whitelisted env, no console window, killed when dropped.
pub fn spawn_harness_child(command: &str, args: &[String], cwd: &str) -> Result<Child, String> {
    let program = crate::utils::exec_resolve::resolve_or_keep(command);
    let mut cmd = Command::new(&program);
    cmd.env_clear().envs(inherited_env_whitelist());
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // Suppress the console window for GUI-launched CLI children.
        #[allow(unused_imports)]
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd.spawn().map_err(|e| {
        crate::ierr!(
            "启动 CLI 工具失败 ({}): {}",
            "Failed to launch the CLI tool ({}): {}",
            program,
            e
        )
    })
}

/// Hard-stop escalation: kill the whole child process tree. Plain `Child::kill`
/// only reaps the direct child — npm shims (`npx.cmd` → `node.exe` → agent) would
/// orphan the grandchildren, so on Windows we walk the tree via `taskkill /T /F`.
/// On Unix, killing the direct child plus dropping the handle (kill_on_drop) is
/// the accepted behavior for these CLIs.
pub async fn kill_child_tree(child: &tokio::sync::Mutex<Child>) {
    let pid = child.lock().await.id();
    #[cfg(windows)]
    if let Some(pid) = pid {
        let mut cmd = Command::new(crate::utils::exec_resolve::resolve_or_keep("taskkill"));
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"]);
        #[allow(unused_imports)]
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
        if let Ok(mut killed) = cmd.spawn() {
            let _ = killed.wait().await;
            return;
        }
        // taskkill itself unavailable (stripped PATH): fall through to direct kill.
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        let mut c = child.lock().await;
        let _ = c.kill().await;
    }
    // Best effort: also reap the direct child handle (no-op when already dead).
    let mut c = child.lock().await;
    let _ = c.kill().await;
}

/// Resolve the concrete (program, args) for an ACP tool spec. `Bare` entries are
/// PATH-resolved via the shared SEC-06 resolver; `Npx` entries run through
/// `npx -y <package>` so the adapter is fetched on first use when not installed.
/// Native-kind specs (ClaudeNative / CodexExec) never reach the ACP adapter
/// directly — the manager rewrites them to the ACP fallback kind first (arms
/// kept exhaustive for safety).
pub fn resolve_acp_command(spec: &HarnessToolSpec) -> Result<(String, Vec<String>), String> {
    match &spec.kind {
        HarnessToolKind::ClaudeNative { .. } | HarnessToolKind::CodexExec { .. } => {
            Err("原生适配器工具不能直接走 ACP 命令（应先回退到 Acp kind）".to_string())
        }
        HarnessToolKind::Acp { command } => match command {
            AcpCommand::Bare { command, args } => Ok((command.clone(), args.clone())),
            AcpCommand::Npx { package, args } => {
                if crate::utils::exec_resolve::resolve_executable("npx").is_none() {
                    return Err(crate::ierr!(
                        "未找到 Node.js (npx)；请先安装 Node.js 20+ 再使用此工具",
                        "Node.js (npx) not found; install Node.js 20+ before using this tool"
                    ));
                }
                let mut full = vec!["-y".to_string(), package.clone()];
                full.extend(args.iter().cloned());
                Ok(("npx".to_string(), full))
            }
        },
    }
}
