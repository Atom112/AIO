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
    /// Session-owned child (ACP / claude); None for exec-per-turn adapters
    /// (codex tracks its current-turn process internally).
    pub child: Option<Arc<tokio::sync::Mutex<Child>>>,
    pub token: CancellationToken,
}

/// Managed state for the harness workbench.
#[derive(Clone)]
pub struct HarnessManager {
    /// Live runtimes keyed by (tool_id, session_row_id).
    pub sessions: DashMap<HarnessSessionKey, Arc<SessionRuntime>>,
    /// Pending approvals keyed by request id (resolved by harness_respond_approval).
    pub approvals: Arc<DashMap<String, oneshot::Sender<ApprovalDecision>>>,
    /// Last driver-reported state per session ("connecting"/"running"/"idle"/
    /// "error") — the emitter task writes it, `running_session_row_ids` reads
    /// it. This is what distinguishes an idle CONNECTED session (spawned by
    /// eager connect, no turn in flight) from a genuinely running turn; the
    /// pre-eager-connect shape (runtime exists ⇒ a turn was sent) no longer
    /// holds, so "live runtime" cannot stand in for "running turn".
    last_states: DashMap<HarnessSessionKey, String>,
}

impl HarnessManager {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
            approvals: Arc::new(DashMap::new()),
            last_states: DashMap::new(),
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
        prefs: crate::plugins::harness::adapter::DriverSpawnPrefs,
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
        // whole session; also tracks the last reported state for the running-turn
        // reconciliation query (live runtime ≠ running turn).
        {
            let app = app.clone();
            let (tool, row) = key.clone();
            let last_states = self.last_states.clone();
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
                        DriverEvent::State { state, error } => {
                            last_states.insert((tool.clone(), row.clone()), state.clone());
                            app.emit(
                                ev::HARNESS_STATE,
                                crate::plugins::harness::model::HarnessStatePayload {
                                    tool_id: tool.clone(),
                                    session_row_id: row.clone(),
                                    state,
                                    error,
                                },
                            )
                        }
                    };
                }
            });
        }

        // Dispatch to the adapter family. Native adapters take priority when
        // their binary exists; without one the bundled ACP fallback serves the
        // tool (the spec is rewritten for the ACP adapter at dispatch).
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
                        spawn_options: spec.spawn_options.clone(),
                        setup: spec.setup.clone(),
                    };
                    Arc::new(crate::plugins::harness::acp::AcpAdapter)
                }
            }
            HarnessToolKind::CodexExec { fallback } => {
                if crate::utils::exec_resolve::resolve_executable("codex").is_some() {
                    Arc::new(crate::plugins::harness::codex::CodexAdapter)
                } else {
                    spec = HarnessToolSpec {
                        id: spec.id,
                        name: spec.name,
                        kind: HarnessToolKind::Acp {
                            command: fallback.clone(),
                        },
                        spawn_options: spec.spawn_options.clone(),
                        setup: spec.setup.clone(),
                    };
                    Arc::new(crate::plugins::harness::acp::AcpAdapter)
                }
            }
        };

        // Tool-bound API key: the key the user stored in AIO's secure store for
        // this tool is injected as the tool's provider env var at spawn (only
        // that one named var, on top of the child env whitelist).
        let mut extra_env: Vec<(String, String)> = Vec::new();
        if let Some(env_var) = &spec.setup.api_key_env {
            if let Ok(Some(key)) = crate::core::secure_store::get(
                app,
                &crate::core::secure_store::accounts::harness_tool_key(tool_id),
            ) {
                if !key.trim().is_empty() {
                    extra_env.push((env_var.clone(), key));
                }
            }
        }

        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<HarnessCommand>();
        let DriverHandle { child } = adapter
            .start(
                DriverSpawn {
                    ctx,
                    spec,
                    workspace_path: workspace_path.to_string(),
                    harness_session_id,
                    permission_mode: prefs.permission_mode,
                    initial_model: prefs.model,
                    initial_thinking: prefs.thinking_level,
                    extra_env,
                    spawn_option_values: prefs.spawn_option_values,
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
        if let Some(child) = &runtime.child {
            kill_child_tree(child).await;
        }
        self.sessions.remove(&key);
        self.last_states.remove(&key);
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

    /// Set a session config option on a RUNNING session (model selector /
    /// thought level / boolean toggle). Silent no-op when the session is not
    /// live — the frontend persists the choice for the next spawn instead.
    pub fn set_config_option(
        &self,
        session_row_id: &str,
        config_id: &str,
        value: serde_json::Value,
    ) {
        for entry in self.sessions.iter() {
            if entry.key().1 == session_row_id {
                let _ = entry.value().cmd_tx.send(HarnessCommand::SetConfigOption {
                    config_id: config_id.to_string(),
                    value,
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
                self.last_states.remove(&key);
                runtime.token.cancel();
                if let Some(child) = &runtime.child {
                    let child = child.clone();
                    tokio::spawn(async move {
                        kill_child_tree(&child).await;
                    });
                }
            }
        }
    }

    /// Session row ids with a turn genuinely in flight (frontend reconciliation
    /// on page mount: stop buttons keep working for sessions streaming while
    /// the user is elsewhere). NOT "live runtimes" — eager connect keeps idle
    /// runtimes alive, so we filter by the last driver-reported state.
    pub fn running_session_row_ids(&self) -> Vec<String> {
        self.sessions
            .iter()
            .filter(|e| {
                self.last_states
                    .get(e.key())
                    .map(|s| s.as_str() == "running")
                    .unwrap_or(false)
            })
            .map(|e| e.key().1.clone())
            .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_runtime() -> Arc<SessionRuntime> {
        let (cmd_tx, _rx) = mpsc::unbounded_channel();
        Arc::new(SessionRuntime {
            cmd_tx,
            child: None,
            token: CancellationToken::new(),
        })
    }

    /// running_session_row_ids reports only sessions whose last driver-reported
    /// state is "running" — a live-but-idle runtime (eager connect) must not
    /// read as a running turn on page-mount reconciliation.
    #[test]
    fn running_ids_filter_by_last_state() {
        let m = HarnessManager::new();
        let running_key: HarnessSessionKey = ("t".into(), "running-row".into());
        let idle_key: HarnessSessionKey = ("t".into(), "idle-row".into());
        let nostate_key: HarnessSessionKey = ("t".into(), "fresh-row".into());
        m.sessions.insert(running_key.clone(), fake_runtime());
        m.sessions.insert(idle_key.clone(), fake_runtime());
        m.sessions.insert(nostate_key.clone(), fake_runtime());
        m.last_states.insert(running_key, "running".to_string());
        m.last_states.insert(idle_key, "idle".to_string());
        // fresh-row: runtime exists, no state yet → not running.

        let ids = m.running_session_row_ids();
        assert_eq!(ids, vec!["running-row".to_string()]);
    }
}
