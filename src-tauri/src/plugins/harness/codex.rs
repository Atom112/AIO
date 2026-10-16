//! OpenAI Codex native driver — the `codex exec --json` adapter (preferred over
//! the ACP fallback when a `codex` CLI is installed).
//!
//! Transport: one process per turn. Each prompt spawns
//! `codex exec --json [-C <dir>] [resume <thread_id>] -` with the prompt piped
//! to stdin (`-` forces stdin — user content never rides argv, so the npm
//! .cmd/.ps1 shim forms stay safe), reads the NDJSON event stream until the
//! process exits, and maps frames to the normalized model.
//!
//! Known boundary (verified from codex-rs source): exec mode forces
//! `approval_policy=never` and REJECTS approval requests server-side — there is
//! no interactive permission surface. Sandboxing/approvals come from the
//! user's `~/.codex/config.toml` instead. Documented in harness.md.
//!
//! Wire reference: codex-rs/exec/src/exec_events.rs (schema verified during
//! bring-up research): thread.started / turn.started / item.{started,updated,
//! completed} / turn.{completed,failed} / error.

use crate::plugins::harness::adapter::{
    spawn_harness_child, DriverCtx, DriverHandle, DriverSpawn, HarnessAdapter, HarnessCommand,
};
use crate::plugins::harness::model::{
    HarnessEvent, HarnessFileChange, HarnessPlanEntry, HarnessUsage,
};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Child;
use tokio::sync::mpsc;

/// The Codex native adapter.
pub struct CodexAdapter;

#[async_trait::async_trait]
impl HarnessAdapter for CodexAdapter {
    async fn start(
        &self,
        spawn: DriverSpawn,
        cmd_rx: mpsc::UnboundedReceiver<HarnessCommand>,
    ) -> Result<DriverHandle, String> {
        CodexDriver::launch(spawn, cmd_rx).await
    }
}

struct CodexDriver {
    ctx: DriverCtx,
    tool_label: String,
    workspace_path: String,
    /// Codex thread id (= exec session id) for `exec resume` across turns.
    thread_id: parking_lot::Mutex<Option<String>>,
    prompt_running: AtomicBool,
    /// The current turn's process (None between turns); cancel kills the tree.
    turn_child: Arc<tokio::sync::Mutex<Option<Child>>>,
    /// Per-turn model (-m flag; initial value from the spawn preference).
    runtime_model: parking_lot::Mutex<Option<String>>,
    /// Per-turn reasoning effort (-c model_reasoning_effort=...).
    runtime_effort: parking_lot::Mutex<Option<String>>,
}

impl CodexDriver {
    async fn launch(
        spawn: DriverSpawn,
        cmd_rx: mpsc::UnboundedReceiver<HarnessCommand>,
    ) -> Result<DriverHandle, String> {
        let tool_label = spawn.spec.name.to_string();
        if crate::utils::exec_resolve::resolve_executable("codex").is_none() {
            return Err(format!(
                "[{tool_label}] 未找到 codex 命令（PATH 中不可解析）"
            ));
        }
        let driver = Arc::new(CodexDriver {
            ctx: spawn.ctx,
            tool_label,
            workspace_path: spawn.workspace_path,
            thread_id: parking_lot::Mutex::new(spawn.harness_session_id),
            prompt_running: AtomicBool::new(false),
            turn_child: Arc::new(tokio::sync::Mutex::new(None)),
            runtime_model: parking_lot::Mutex::new(spawn.initial_model),
            runtime_effort: parking_lot::Mutex::new(spawn.initial_thinking),
        });
        driver.ctx.emit_state("idle", None);
        // Synthesized config options: model selector (preset list — exec mode
        // has no dynamic model/list) + effort selector; changes via
        // SetConfigOption store for the next per-turn spawn.
        driver.ctx.emit_event(HarnessEvent::ConfigOptions {
            options: codex_config_options(
                driver.runtime_model.lock().clone(),
                driver.runtime_effort.lock().clone(),
            ),
        });
        let d = driver.clone();
        tokio::spawn(async move {
            d.command_loop(cmd_rx).await;
        });
        // exec-per-turn adapter: no session-owned process; the driver tracks
        // (and reaps) its current-turn child itself.
        Ok(DriverHandle { child: None })
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
                    let d = self.clone();
                    tokio::spawn(async move {
                        d.run_turn(text).await;
                    });
                }
                HarnessCommand::CancelTurn => {
                    let child = self.turn_child.lock().await.take();
                    if let Some(mut child) = child {
                        // Kill the whole tree; the turn task's reader then breaks
                        // and emits the cancelled TurnEnded exactly once.
                        let pid = child.id();
                        #[cfg(windows)]
                        if let Some(pid) = pid {
                            let mut cmd = tokio::process::Command::new(
                                crate::utils::exec_resolve::resolve_or_keep("taskkill"),
                            );
                            cmd.args(["/PID", &pid.to_string(), "/T", "/F"]);
                            #[allow(unused_imports)]
                            use std::os::windows::process::CommandExt;
                            const CREATE_NO_WINDOW: u32 = 0x08000000;
                            cmd.creation_flags(CREATE_NO_WINDOW);
                            if let Ok(mut killed) = cmd.spawn() {
                                let _ = killed.wait().await;
                            } else {
                                let _ = child.kill().await;
                            }
                        }
                        #[cfg(not(windows))]
                        {
                            let _ = pid;
                            let _ = child.kill().await;
                        }
                        #[cfg(windows)]
                        {
                            let _ = child.kill().await;
                        }
                    }
                }
                HarnessCommand::SetPermissionMode { .. } => {
                    // exec mode has no runtime permission surface (approval
                    // policy is fixed by ~/.codex/config.toml at process start).
                    tracing::debug!(
                        "[harness:{}] 忽略 SetPermissionMode（codex exec 无此界面）",
                        self.tool_label
                    );
                }
                HarnessCommand::SetConfigOption { config_id, value } => {
                    let Some(val) = value.as_str() else {
                        continue;
                    };
                    match config_id.as_str() {
                        "model" => *self.runtime_model.lock() = Some(val.to_string()),
                        "effort" => *self.runtime_effort.lock() = Some(val.to_string()),
                        _ => {
                            tracing::debug!(
                                "[harness:{}] 忽略未知 config 选项 {config_id}",
                                self.tool_label
                            );
                            continue;
                        }
                    }
                    self.ctx.emit_event(HarnessEvent::ConfigOptions {
                        options: codex_config_options(
                            self.runtime_model.lock().clone(),
                            self.runtime_effort.lock().clone(),
                        ),
                    });
                }
            }
        }
    }

    /// One full turn: spawn `codex exec --json ... -`, pipe the prompt to
    /// stdin, stream frames until the process exits, then emit the terminal
    /// event exactly once (end_turn / error / cancelled).
    async fn run_turn(self: Arc<Self>, text: String) {
        let mut args: Vec<String> = vec![
            "exec".to_string(),
            "--json".to_string(),
            // Workspaces need not be git repos (AIO registers any directory).
            "--skip-git-repo-check".to_string(),
        ];
        if let Some(tid) = self.thread_id.lock().clone() {
            args.push("resume".to_string());
            args.push(tid);
        }
        // Per-turn model & effort (SetConfigOption-stored runtime preferences).
        if let Some(model) = self.runtime_model.lock().clone() {
            args.push("-m".to_string());
            args.push(model);
        }
        if let Some(effort) = self.runtime_effort.lock().clone() {
            args.push("-c".to_string());
            args.push(format!("model_reasoning_effort={effort}"));
        }
        args.push("-C".to_string());
        args.push(self.workspace_path.clone());
        // `-` forces the prompt to be read from stdin: user content never
        // rides argv (BatBadBut surface eliminated for .cmd shim forms).
        args.push("-".to_string());

        let mut child = match spawn_harness_child("codex", &args, &self.workspace_path) {
            Ok(c) => c,
            Err(e) => {
                self.ctx.emit_event(HarnessEvent::Error { message: e });
                return;
            }
        };
        let mut stdin = match child.stdin.take() {
            Some(s) => s,
            None => {
                let _ = child.kill().await;
                self.ctx.emit_event(HarnessEvent::Error {
                    message: "codex 子进程 stdin 不可用".to_string(),
                });
                return;
            }
        };
        let stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                let _ = child.kill().await;
                self.ctx.emit_event(HarnessEvent::Error {
                    message: "codex 子进程 stdout 不可用".to_string(),
                });
                return;
            }
        };
        let stderr = child.stderr.take();
        if let Some(stderr) = stderr {
            let label = self.tool_label.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    tracing::debug!("[harness:{label}] stderr: {line}");
                }
            });
        }
        // Pipe the prompt then close stdin (EOF signals "turn submitted").
        let send_failed =
            stdin.write_all(text.as_bytes()).await.is_err() || stdin.flush().await.is_err();
        drop(stdin);
        if send_failed {
            let _ = child.wait().await;
            self.ctx.emit_event(HarnessEvent::Error {
                message: "写入 codex stdin 失败".to_string(),
            });
            return;
        }

        self.prompt_running.store(true, Ordering::SeqCst);
        *self.turn_child.lock().await = Some(child);
        self.ctx.emit_event(HarnessEvent::TurnStarted);
        self.ctx.emit_state("running", None);

        // Terminal bookkeeping exactly once per turn.
        let finished = AtomicBool::new(false);
        let close_turn = |stop_reason: String, error: Option<String>| {
            if finished.swap(true, Ordering::SeqCst) {
                return;
            }
            self.ctx
                .emit_event(HarnessEvent::TurnEnded { stop_reason, error });
            self.ctx.emit_state("idle", None);
        };

        let mut lines = BufReader::new(stdout).lines();
        loop {
            let line = tokio::select! {
                _ = self.ctx.token.cancelled() => break,
                l = lines.next_line() => match l {
                    Ok(Some(l)) => l,
                    Ok(None) => break,
                    Err(_) => break,
                },
            };
            let line = line.trim_end_matches('\r');
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                tracing::debug!("[harness:{}] 非 JSON 行: {line}", self.tool_label);
                continue;
            };
            match v.get("type").and_then(Value::as_str) {
                Some("thread.started") => {
                    let tid = v
                        .get("thread_id")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    if let Some(tid) = tid {
                        *self.thread_id.lock() = Some(tid.clone());
                        self.ctx.set_harness_session_id(&tid);
                        self.ctx.emit_event(HarnessEvent::SessionInfo {
                            harness_session_id: Some(tid),
                            model: None,
                        });
                    }
                }
                Some("turn.completed") => {
                    let usage = usage_from_turn_completed(&v);
                    self.ctx.emit_event(HarnessEvent::UsageUpdate { usage });
                    close_turn("end_turn".to_string(), None);
                }
                Some("turn.failed") => {
                    let msg = v
                        .pointer("/error/message")
                        .and_then(Value::as_str)
                        .unwrap_or("codex 回合失败");
                    close_turn("error".to_string(), Some(msg.to_string()));
                }
                Some("error") => {
                    let msg = v
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("未知错误");
                    close_turn("error".to_string(), Some(msg.to_string()));
                }
                Some("item.started") | Some("item.updated") | Some("item.completed") => {
                    for event in exec_item_events(&v) {
                        self.ctx.emit_event(event);
                    }
                }
                Some("turn.started") | Some(_) | None => {}
            }
        }

        // Reap the process and close the turn if nothing terminal arrived yet
        // (killed mid-stream / process crash → cancelled or error).
        {
            let mut slot = self.turn_child.lock().await;
            if let Some(mut child) = slot.take() {
                let _ = child.wait().await;
            }
        }
        self.prompt_running.store(false, Ordering::SeqCst);
        if self.ctx.token.is_cancelled() {
            close_turn("cancelled".to_string(), None);
        } else {
            close_turn("error".to_string(), Some("codex 进程提前退出".to_string()));
        }
    }
}

/// Synthesize the config-option set for the Codex native driver: a model
/// selector (preset list — exec mode has no dynamic model/list surface; the
/// app-server-only model/list method is deliberately avoided) and an effort
/// selector mapping to the `-c model_reasoning_effort=` per-turn override.
pub fn codex_config_options(
    model: Option<String>,
    effort: Option<String>,
) -> Vec<crate::plugins::harness::model::HarnessConfigOption> {
    use crate::plugins::harness::model::{HarnessConfigOption, HarnessConfigValue};
    let model_values = vec![
        HarnessConfigValue {
            id: "gpt-5.2-codex".into(),
            name: Some("GPT-5.2 Codex".into()),
        },
        HarnessConfigValue {
            id: "gpt-5.2".into(),
            name: Some("GPT-5.2".into()),
        },
        HarnessConfigValue {
            id: "o4-mini".into(),
            name: Some("o4-mini".into()),
        },
    ];
    let effort_values = vec![
        HarnessConfigValue {
            id: "minimal".into(),
            name: None,
        },
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

/// `turn.completed.usage` → normalized usage snapshot.
fn usage_from_turn_completed(v: &Value) -> HarnessUsage {
    let u = v.get("usage").cloned().unwrap_or(Value::Null);
    let get = |key: &str| u.get(key).and_then(Value::as_u64);
    HarnessUsage {
        input_tokens: get("input_tokens"),
        output_tokens: get("output_tokens"),
        context_tokens: get("input_tokens"),
        context_window: None,
        cost_usd: None,
    }
}

/// One `item.*` frame → normalized events. The item is identified by its own
/// `id` (e.g. "item_1") which doubles as the tool-call id in our model.
fn exec_item_events(v: &Value) -> Vec<HarnessEvent> {
    let Some(item) = v.get("item") else {
        return Vec::new();
    };
    let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
    let id = || {
        item.get("id")
            .and_then(Value::as_str)
            .unwrap_or("codex_item")
            .to_string()
    };
    let completed = v.get("type").and_then(Value::as_str) == Some("item.completed");
    match kind {
        "agent_message" if completed => {
            let text = item.get("text").and_then(Value::as_str).unwrap_or("");
            (!text.is_empty())
                .then(|| HarnessEvent::TextDelta {
                    text: text.to_string(),
                })
                .into_iter()
                .collect()
        }
        "reasoning" if completed => {
            let text = item.get("text").and_then(Value::as_str).unwrap_or("");
            (!text.is_empty())
                .then(|| HarnessEvent::ThinkingDelta {
                    text: text.to_string(),
                })
                .into_iter()
                .collect()
        }
        "command_execution" => vec![if completed {
            let exit = item.get("exit_code").and_then(Value::as_i64);
            let ok = exit == Some(0);
            let output = item
                .get("aggregated_output")
                .and_then(Value::as_str)
                .map(str::to_string);
            HarnessEvent::ToolCallEnd {
                tool_call_id: id(),
                ok,
                content: if ok { output.clone() } else { None },
                error: if ok {
                    None
                } else {
                    output.or(Some("命令失败".to_string()))
                },
                file_changes: Vec::new(),
            }
        } else {
            let command = item.get("command").and_then(Value::as_str).unwrap_or("");
            HarnessEvent::ToolCallStart {
                tool_call_id: id(),
                name: Some("command_execution".to_string()),
                title: Some(command.to_string()),
                tool_kind: Some("execute".to_string()),
                arguments: Some(command.to_string()),
            }
        }],
        "file_change" => vec![if completed {
            let changes = item
                .get("changes")
                .and_then(Value::as_array)
                .map(|arr| {
                    arr.iter()
                        .filter_map(|c| {
                            Some(HarnessFileChange {
                                file_path: c.get("path")?.as_str()?.to_string(),
                                // exec events carry kind but not patch text.
                                action: match c.get("kind").and_then(Value::as_str) {
                                    Some("add") => "create".to_string(),
                                    Some("delete") => "delete".to_string(),
                                    _ => "modify".to_string(),
                                },
                                diff: String::new(),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            HarnessEvent::ToolCallEnd {
                tool_call_id: id(),
                ok: item.get("status").and_then(Value::as_str) != Some("failed"),
                content: None,
                error: None,
                file_changes: changes,
            }
        } else {
            HarnessEvent::ToolCallStart {
                tool_call_id: id(),
                name: Some("file_change".to_string()),
                title: Some("文件修改".to_string()),
                tool_kind: Some("edit".to_string()),
                arguments: None,
            }
        }],
        "mcp_tool_call" => vec![if completed {
            let err = item
                .pointer("/error/message")
                .and_then(Value::as_str)
                .map(str::to_string);
            HarnessEvent::ToolCallEnd {
                tool_call_id: id(),
                ok: err.is_none(),
                content: item.get("result").map(|r| r.to_string()),
                error: err,
                file_changes: Vec::new(),
            }
        } else {
            HarnessEvent::ToolCallStart {
                tool_call_id: id(),
                name: Some(
                    item.get("tool")
                        .and_then(Value::as_str)
                        .unwrap_or("mcp_tool")
                        .to_string(),
                ),
                title: item
                    .pointer("/server")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                tool_kind: None,
                arguments: item.get("arguments").map(|a| a.to_string()),
            }
        }],
        "web_search" => vec![if completed {
            HarnessEvent::ToolCallEnd {
                tool_call_id: id(),
                ok: true,
                content: None,
                error: None,
                file_changes: Vec::new(),
            }
        } else {
            HarnessEvent::ToolCallStart {
                tool_call_id: id(),
                name: Some("web_search".to_string()),
                title: item
                    .get("query")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                tool_kind: Some("fetch".to_string()),
                arguments: None,
            }
        }],
        "todo_list" => {
            let Some(items) = item.get("items").and_then(Value::as_array) else {
                return Vec::new();
            };
            vec![HarnessEvent::PlanUpdate {
                entries: items
                    .iter()
                    .map(|e| HarnessPlanEntry {
                        content: e
                            .get("text")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        priority: "medium".to_string(),
                        status: if e.get("completed").and_then(Value::as_bool).unwrap_or(false) {
                            "completed".to_string()
                        } else {
                            "pending".to_string()
                        },
                    })
                    .collect(),
            }]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Literal frames from the documented exec --json schema (research-verified
    /// against codex-rs source): mapping must produce the exact event shapes.
    #[test]
    fn maps_thread_and_message_frames() {
        assert!(exec_item_events(&json!({
            "type": "item.started", "item": { "id": "item_0", "type": "reasoning", "text": "" }
        }))
        .is_empty());
        match exec_item_events(&json!({
            "type": "item.completed",
            "item": { "id": "item_0", "type": "reasoning", "text": "think" }
        }))
        .as_slice()
        {
            [HarnessEvent::ThinkingDelta { text }] => assert_eq!(text, "think"),
            other => panic!("unexpected events: {other:?}"),
        }
        match exec_item_events(&json!({
            "type": "item.completed",
            "item": { "id": "item_5", "type": "agent_message", "text": "Fixed." }
        }))
        .as_slice()
        {
            [HarnessEvent::TextDelta { text }] => assert_eq!(text, "Fixed."),
            other => panic!("unexpected events: {other:?}"),
        }
    }

    #[test]
    fn maps_command_execution_lifecycle() {
        match exec_item_events(&json!({
            "type": "item.started",
            "item": { "id": "item_1", "type": "command_execution", "command": "cargo test", "status": "in_progress" }
        }))
        .as_slice()
        {
            [HarnessEvent::ToolCallStart { tool_call_id, title, .. }] => {
                assert_eq!(tool_call_id, "item_1");
                assert_eq!(title.as_deref(), Some("cargo test"));
            }
            other => panic!("unexpected events: {other:?}"),
        }
        match exec_item_events(&json!({
            "type": "item.completed",
            "item": { "id": "item_1", "type": "command_execution", "command": "cargo test",
                      "aggregated_output": "ok. 3 passed", "exit_code": 0, "status": "completed" }
        }))
        .as_slice()
        {
            [HarnessEvent::ToolCallEnd { ok, content, .. }] => {
                assert!(ok);
                assert_eq!(content.as_deref(), Some("ok. 3 passed"));
            }
            other => panic!("unexpected events: {other:?}"),
        }
        match exec_item_events(&json!({
            "type": "item.completed",
            "item": { "id": "item_2", "type": "command_execution", "command": "bad",
                      "aggregated_output": "boom", "exit_code": 1, "status": "completed" }
        }))
        .as_slice()
        {
            [HarnessEvent::ToolCallEnd { ok, error, .. }] => {
                assert!(!ok);
                assert_eq!(error.as_deref(), Some("boom"));
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    /// file_change items map to ToolCallEnd with action mapping; exec events
    /// carry no patch text (diff stays empty — display shows the path label).
    #[test]
    fn maps_file_change_items() {
        match exec_item_events(&json!({
            "type": "item.completed",
            "item": { "id": "item_3", "type": "file_change", "status": "completed",
                      "changes": [ { "path": "src/main.rs", "kind": "update" } ] }
        }))
        .as_slice()
        {
            [HarnessEvent::ToolCallEnd {
                file_changes, ok, ..
            }] => {
                assert!(ok);
                assert_eq!(file_changes.len(), 1);
                assert_eq!(file_changes[0].file_path, "src/main.rs");
                assert_eq!(file_changes[0].action, "modify");
                assert!(file_changes[0].diff.is_empty());
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    /// todo_list items map to the plan/task board.
    #[test]
    fn maps_todo_list_to_plan() {
        match exec_item_events(&json!({
            "type": "item.completed",
            "item": { "id": "item_4", "type": "todo_list",
                      "items": [ { "text": "fix bug", "completed": true }, { "text": "add test", "completed": false } ] }
        }))
        .as_slice()
        {
            [HarnessEvent::PlanUpdate { entries }] => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].status, "completed");
                assert_eq!(entries[1].status, "pending");
            }
            other => panic!("unexpected events: {other:?}"),
        }
    }

    /// turn.completed usage extraction.
    #[test]
    fn maps_turn_usage() {
        let u = usage_from_turn_completed(&json!({
            "type": "turn.completed",
            "usage": { "input_tokens": 1234, "output_tokens": 567, "cached_input_tokens": 0 }
        }));
        assert_eq!(u.input_tokens, Some(1234));
        assert_eq!(u.output_tokens, Some(567));
        assert_eq!(u.context_tokens, Some(1234));
    }
}

#[cfg(test)]
mod live_tests {
    use super::*;
    use crate::plugins::harness::adapter::{DriverCtx, DriverSpawn, HarnessEmitter};
    use crate::plugins::harness::model::DriverEvent;
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

    /// Live roundtrip against a locally installed `codex` CLI.
    /// Ignored by default — needs `codex` on PATH, logged in (ChatGPT or API key).
    /// `cargo test --manifest-path src-tauri/Cargo.toml codex_live -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "live: requires `codex` on PATH with a logged-in session"]
    async fn codex_live_roundtrip() {
        if crate::utils::exec_resolve::resolve_executable("codex").is_none() {
            eprintln!("[live] codex 未安装，跳过真实联调");
            return;
        }
        let ws = std::env::temp_dir().join(format!("aio-codex-live-{}", std::process::id()));
        std::fs::create_dir_all(&ws).unwrap();
        let workspace_path = store::canonical_workspace_path(&ws).unwrap();

        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::core::db::create_base_schema(&conn).unwrap();
        crate::core::migrate::run_migrations(&conn, crate::core::db::CHAT_DB_MIGRATIONS).unwrap();
        const ROW_ID: &str = "codex-live-row";
        conn.execute(
            "INSERT INTO harness_sessions (id, tool_id, workspace_path, title, created_at, updated_at)
             VALUES (?1, 'codex', ?2, 'live test', 'now', 'now')",
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
            key: ("codex".to_string(), ROW_ID.to_string()),
            token: token.clone(),
            events: event_tx,
            approvals: Arc::new(dashmap::DashMap::new()),
        };

        let spec = crate::plugins::harness::registry::lookup("codex").unwrap();
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<HarnessCommand>();
        let _handle = CodexDriver::launch(
            DriverSpawn {
                ctx,
                spec,
                workspace_path: workspace_path.clone(),
                harness_session_id: None,
                permission_mode: None,
                initial_model: None,
                initial_thinking: None,
            },
            cmd_rx,
        )
        .await
        .expect("codex 驱动启动");

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
        let _ = std::fs::remove_dir_all(&ws);
        assert!(ended.is_ok(), "240s 内未收到 TurnEnded");
        assert!(turn_error.is_none(), "驱动器报错: {}", turn_error.unwrap());
        assert!(got_text, "回合内未收到任何文本流");
    }
}
