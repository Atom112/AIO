//! Harness manager: owned runtimes for live harness sessions (child process,
//! cancellation token, command sender) plus the pending-approval resolution map.
//!
//! Lifecycle semantics mirror the chat pipeline's StreamManager:
//! - `ensure_session` spawns a driver and awaits its handshake (idempotent —
//!   repeated calls reuse the live runtime; a spawn race kills the loser).
//! - `stop_session` is two-phase: a protocol-level cancel first (the agent
//!   answers the pending prompt with stopReason=cancelled), then after a grace
//!   window the token is cancelled and the child process tree is killed.
//! - On failure the runtime is dropped so the next prompt re-spawns and resumes
//!   from the persisted harness session id.

use crate::core::events as ev;
use crate::ierr;
use crate::plugins::harness::adapter::{
    kill_child_tree, DriverHandle, DriverSpawn, HarnessAdapter, HarnessCommand,
};
use crate::plugins::harness::model::{ApprovalDecision, DriverEvent, HarnessSessionKey};
use crate::plugins::harness::registry::{lookup_any, HarnessToolKind, HarnessToolSpec};
use dashmap::DashMap;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use tokio::process::Child;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

/// One live harness session runtime.
pub struct SessionRuntime {
    pub cmd_tx: mpsc::UnboundedSender<HarnessCommand>,
    pub child: Arc<tokio::sync::Mutex<Child>>,
    pub token: CancellationToken,
}

/// Managed state for the harness workbench.
#[derive(Clone)]
pub struct HarnessManager {
    /// Live runtimes keyed by (tool_id, session_row_id).
    pub sessions: DashMap<HarnessSessionKey, Arc<SessionRuntime>>,
    /// Pending approvals keyed by request id (resolved by harness_respond_approval).
    pub approvals: Arc<DashMap<String, oneshot::Sender<ApprovalDecision>>>,
}

impl HarnessManager {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            approvals: Arc::new(DashMap::new()),
        }
    }

    /// Resolve a user approval decision (returns false when the request expired).
    pub fn respond_approval(&self, request_id: &str, option_id: &str) -> bool {
        match self.approvals.remove(request_id) {
            Some((_, tx)) => {
                let _ = tx.send(ApprovalDecision {
                    option_id: Some(option_id.to_string()),
                    cancelled: false,
                });
                true
            }
            None => false,
        }
    }

    /// Get (or spawn) the runtime for a harness session. Spawning includes the
    /// full native handshake; on error nothing is registered.
    pub async fn ensure_session(
        &self,
        app: &AppHandle,
        tool_id: &str,
        session_row_id: &str,
        workspace_path: &str,
        harness_session_id: Option<String>,
        permission_mode: Option<String>,
    ) -> Result<Arc<SessionRuntime>, String> {
        let key: HarnessSessionKey = (tool_id.to_string(), session_row_id.to_string());
        if let Some(rt) = self.sessions.get(&key) {
            return Ok(rt.clone());
        }
        let mut spec = lookup_any(app, tool_id).ok_or_else(|| {
            ierr!(
                "未知的 harness 工具: {tool_id}",
                "Unknown harness tool: {tool_id}"
            )
        })?;

        let token = CancellationToken::new();
        let (event_tx, mut event_rx) = mpsc::unbounded_channel::<DriverEvent>();
        let ctx = crate::plugins::harness::adapter::DriverCtx {
            emitter: std::sync::Arc::new(crate::plugins::harness::adapter::TauriHarnessEmitter {
                app: app.clone(),
            }),
            client_version: app.package_info().version.to_string(),
            key: key.clone(),
            token: token.clone(),
            events: event_tx,
            approvals: self.approvals.clone(),
        };

        // Emitter task: serializes driver events into Tauri events. Lives for the
        // whole session; the emitter owns nothing else.
        {
            let app = app.clone();
            let (tool, row) = key.clone();
            tokio::spawn(async move {
                while let Some(ev) = event_rx.recv().await {
                    let _ = match ev {
                        DriverEvent::Event(event) => app.emit(
                            ev::HARNESS_EVENT,
                            crate::plugins::harness::model::HarnessEventPayload {
                                tool_id: tool.clone(),
                                session_row_id: row.clone(),
                                event,
                            },
                        ),
                        DriverEvent::State { state, error } => app.emit(
                            ev::HARNESS_STATE,
                            crate::plugins::harness::model::HarnessStatePayload {
                                tool_id: tool.clone(),
                                session_row_id: row.clone(),
                                state,
                                error,
                            },
                        ),
                    };
                }
            });
        }

        // Dispatch to the adapter family. Claude Code prefers its native driver
        // when a real binary exists; without one the bundled ACP fallback serves
        // the tool (registry rewrites the spec for the ACP adapter).
        let adapter: Arc<dyn HarnessAdapter> = match &spec.kind {
            HarnessToolKind::Acp { .. } => Arc::new(crate::plugins::harness::acp::AcpAdapter),
            HarnessToolKind::ClaudeNative { fallback } => {
                if crate::plugins::harness::claude::detect_native_binary().is_some() {
                    Arc::new(crate::plugins::harness::claude::ClaudeAdapter)
                } else {
                    spec = HarnessToolSpec {
                        id: spec.id,
                        name: spec.name,
                        kind: HarnessToolKind::Acp {
                            command: fallback.clone(),
                        },
                    };
                    Arc::new(crate::plugins::harness::acp::AcpAdapter)
                }
            }
        };

        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<HarnessCommand>();
        let DriverHandle { child } = adapter
            .start(
                DriverSpawn {
                    ctx,
                    spec,
                    workspace_path: workspace_path.to_string(),
                    harness_session_id,
                    permission_mode,
                },
                cmd_rx,
            )
            .await?;

        let runtime = Arc::new(SessionRuntime {
            cmd_tx,
            child,
            token,
        });
        // Spawn-race guard: if another caller won the insert, drop ours (kill_on_drop
        // reaps the loser child) and reuse the winner.
        match self.sessions.entry(key.clone()) {
            dashmap::mapref::entry::Entry::Occupied(existing) => {
                let existing = existing.get().clone();
                drop(runtime);
                Ok(existing)
            }
            dashmap::mapref::entry::Entry::Vacant(slot) => {
                slot.insert(runtime.clone());
                Ok(runtime)
            }
        }
    }

    /// Two-phase stop: protocol-level cancel, grace window, then hard kill.
    /// The runtime is removed — the next prompt re-spawns and resumes.
    pub async fn stop_session(&self, session_row_id: &str) -> Result<(), String> {
        let key = self
            .sessions
            .iter()
            .find(|e| e.key().1 == session_row_id)
            .map(|e| e.key().clone());
        let Some(key) = key else {
            return Ok(()); // not running — nothing to do
        };
        let runtime = self.sessions.get(&key).map(|e| e.value().clone());
        let Some(runtime) = runtime else {
            return Ok(());
        };
        let _ = runtime.cmd_tx.send(HarnessCommand::CancelTurn);
        // Grace window: a cooperative agent answers the pending prompt with
        // stopReason=cancelled and emits TurnEnded itself.
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        runtime.token.cancel();
        kill_child_tree(&runtime.child).await;
        self.sessions.remove(&key);
        Ok(())
    }

    /// Switch the permission mode of a RUNNING session (Claude Code native:
    /// set_permission_mode control frame; other adapters ignore it). Silent
    /// no-op when the session is not live — the frontend stores the choice and
    /// applies it at the next spawn instead.
    pub fn set_permission_mode(&self, session_row_id: &str, mode: &str) {
        for entry in self.sessions.iter() {
            if entry.key().1 == session_row_id {
                let _ = entry
                    .value()
                    .cmd_tx
                    .send(HarnessCommand::SetPermissionMode {
                        mode: mode.to_string(),
                    });
                return;
            }
        }
    }

    /// Hard cleanup (app window destroyed): cancel every live session without
    /// awaiting (the window is going away; kill_on_drop + taskkill fire-and-forget).
    pub fn shutdown_all(&self) {
        let keys: Vec<HarnessSessionKey> = self.sessions.iter().map(|e| e.key().clone()).collect();
        for key in keys {
            if let Some((_, runtime)) = self.sessions.remove(&key) {
                runtime.token.cancel();
                let child = runtime.child.clone();
                tokio::spawn(async move {
                    kill_child_tree(&child).await;
                });
            }
        }
    }

    /// Live session row ids (frontend reconciliation on page mount: sessions may
    /// keep running while the user is elsewhere in the app).
    pub fn running_session_row_ids(&self) -> Vec<String> {
        self.sessions.iter().map(|e| e.key().1.clone()).collect()
    }
}

impl Default for HarnessManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Reconcile running sessions on webview reload (mirrors the chat stream
/// reconciliation): after a page remount the frontend asks which harness
/// sessions are still streaming so stop buttons keep working.
pub fn list_running_harness_sessions(app: &AppHandle) -> Vec<String> {
    let manager = app.state::<HarnessManager>();
    manager.running_session_row_ids()
}
