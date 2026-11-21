//! One-shot ACP `session/list` probe used by the "import existing session"
//! dialog. The probe opens its own initialize-only connection and never creates
//! or prompts a session; the adopted topic later connects normally (and
//! `session/load` replays its transcript).

use crate::plugins::harness::adapter::{
    kill_child_tree, resolve_acp_command, spawn_harness_child_env,
};
use crate::plugins::harness::model::{HarnessAgentSessionInfo, HarnessAgentSessionList};
use crate::plugins::harness::registry::HarnessToolSpec;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, ChildStdout};
use tokio::sync::Mutex;

/// Probe an ACP tool's `session/list` and return its adoptable sessions.
///
/// This is a one-shot connection (initialize → session/list → kill): it never
/// creates or prompts a session, so it is safe to call from the import dialog
/// while another topic runtime is live. Native adapters (Claude/Codex) never
/// reach this function — the command layer reports them as unsupported.
pub async fn list_agent_sessions(
    spec: &HarnessToolSpec,
    cwd: &str,
) -> Result<HarnessAgentSessionList, String> {
    let (program, args) = resolve_acp_command(spec)?;
    let mut child = spawn_harness_child_env(&program, &args, cwd, &[])?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "子进程 stdin 不可用".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "子进程 stdout 不可用".to_string())?;
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(_line)) = lines.next_line().await {}
        });
    }
    let mut lines = BufReader::new(stdout).lines();

    let result: Result<HarnessAgentSessionList, String> = async {
        write_json_line(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": 1,
                    "clientCapabilities": { "subagents": {} },
                    "clientInfo": { "name": "AIO", "title": "AIO", "version": env!("CARGO_PKG_VERSION") }
                }
            }),
        )
        .await?;
        let init = read_json_response(
            &mut lines,
            &mut stdin,
            1,
            std::time::Duration::from_secs(60),
        )
        .await?;
        let list_supported = init
            .pointer("/result/agentCapabilities/sessionCapabilities/list")
            .map(|v| !v.is_null())
            .unwrap_or(false);
        // Import is only meaningful when the agent can replay the transcript
        // (`session/load`); a list-only agent (e.g. DSH today) would create a
        // topic with no visible history.
        let load_supported = init
            .pointer("/result/agentCapabilities/loadSession")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !list_supported || !load_supported {
            return Ok(HarnessAgentSessionList {
                supported: false,
                sessions: Vec::new(),
            });
        }
        write_json_line(
            &mut stdin,
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "session/list",
                "params": { "cwd": cwd }
            }),
        )
        .await?;
        let resp = read_json_response(
            &mut lines,
            &mut stdin,
            2,
            std::time::Duration::from_secs(60),
        )
        .await?;
        let mut sessions = resp
            .pointer("/result/sessions")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| {
                        Some(HarnessAgentSessionInfo {
                            session_id: s.get("sessionId")?.as_str()?.to_string(),
                            title: s
                                .get("title")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            updated_at: s
                                .get("updatedAt")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            message_count: s
                                .pointer("/_meta/messageCount")
                                .and_then(Value::as_u64),
                            imported: false,
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        // Agents may ignore the cwd filter; keep the list newest-first.
        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(HarnessAgentSessionList {
            supported: true,
            sessions,
        })
    }
    .await;

    kill_child_tree(&Mutex::new(child)).await;
    result
}

/// Write one NDJSON JSON-RPC message.
async fn write_json_line(stdin: &mut ChildStdin, value: &Value) -> Result<(), String> {
    let mut line = value.to_string();
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())
}

/// Read until the response with `id`, answering unknown agent→client requests
/// with `-32601` so a setup probe cannot hang the agent.
async fn read_json_response(
    lines: &mut tokio::io::Lines<BufReader<ChildStdout>>,
    stdin: &mut ChildStdin,
    id: u64,
    budget: std::time::Duration,
) -> Result<Value, String> {
    let read = async {
        loop {
            let line = match lines.next_line().await {
                Ok(Some(line)) => line,
                Ok(None) => return Err("agent 进程已退出".to_string()),
                Err(e) => return Err(e.to_string()),
            };
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if let Some(method) = value.get("method").and_then(Value::as_str) {
                if let Some(request_id) = value.get("id") {
                    let reply = json!({
                        "jsonrpc": "2.0",
                        "id": request_id,
                        "error": { "code": -32601, "message": format!("method not found: {method}") }
                    });
                    let _ = write_json_line(stdin, &reply).await;
                }
                continue;
            }
            if value.get("id").and_then(Value::as_u64) != Some(id) {
                continue;
            }
            if let Some(error) = value.get("error") {
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("未知错误");
                let detail = error
                    .get("data")
                    .filter(|d| !d.is_null())
                    .map(|d| d.to_string())
                    .unwrap_or_default();
                return Err(if detail.is_empty() {
                    message.to_string()
                } else {
                    format!("{message}: {detail}")
                });
            }
            return Ok(value);
        }
    };
    tokio::time::timeout(budget, read)
        .await
        .map_err(|_| "等待 agent 响应超时".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::harness::registry;

    /// Live `session/list` probe against omp (no model call, no session
    /// creation). Verifies the import dialog's data source.
    #[tokio::test]
    #[ignore = "live: requires `omp` on PATH with configured credentials"]
    async fn acp_live_omp_session_list() {
        if crate::utils::exec_resolve::resolve_executable("omp").is_none() {
            eprintln!("[live] omp 未安装，跳过真实联调");
            return;
        }
        let cwd = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("repo root");
        let spec = registry::lookup("omp").expect("omp 必须在注册表内");
        let list = list_agent_sessions(&spec, &cwd.to_string_lossy())
            .await
            .expect("session/list");
        assert!(list.supported, "omp 应声明 sessionCapabilities.list");
        assert!(!list.sessions.is_empty(), "应列出至少一个 omp 会话");
        assert!(list.sessions.iter().all(|s| !s.session_id.is_empty()));
    }
}
