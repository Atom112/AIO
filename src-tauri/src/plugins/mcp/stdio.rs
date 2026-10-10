//! MCP stdio transport
//!
//! Spawns a child process and exchanges JSON-RPC 2.0 messages over stdin/stdout (one JSON per line).
//! The child process does not inherit the system env; only whitelisted env vars are passed (to prevent secret leakage).

use super::connection::{McpConnection, McpTransport};
use super::error::{McpError, McpResult};
use crate::core::events as ev;
use crate::core::models::{
    GetPromptResult, McpPrompt, McpResource, McpServerConfig, McpServerInfo, McpStatus,
    ReadResourceResult, ToolResult, ToolSpec,
};
use crate::core::secure_store;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

pub struct StdioPlugin;

/// Whitelist of system env vars passed through to the MCP stdio child process.
/// The full host environment is no longer inherited; only node/npm/npx and other
/// essentials for running common MCP servers, plus common proxy/locale settings,
/// are allowed. User-configured env for the server can append to/override this list.
fn inherited_env_whitelist() -> std::collections::HashMap<String, String> {
    const KEYS: &[&str] = &[
        "PATH",
        "HOME",
        "USER",
        "USERNAME",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "TMPDIR",
        "TMP",
        "TEMP",
        "APPDATA",
        "LOCALAPPDATA",
        "SYSTEMROOT",
        "PATHEXT",
        "ComSpec",
        "http_proxy",
        "https_proxy",
        "all_proxy",
        "no_proxy",
        "NO_PROXY",
    ];
    let mut out = std::collections::HashMap::new();
    for k in KEYS {
        if let Some(v) = std::env::var_os(k) {
            out.insert(k.to_string(), v.to_string_lossy().into_owned());
        }
    }
    out
}

#[async_trait]
impl super::McpServerPlugin for StdioPlugin {
    fn identifier(&self) -> &'static str {
        "stdio"
    }

    async fn start(&self, app: AppHandle, config: &McpServerConfig) -> McpResult<McpConnection> {
        let (command, args, env_raw, cwd) = match &config.transport {
            crate::core::models::McpTransport::Stdio {
                command,
                args,
                env,
                cwd,
            } => (command.clone(), args.clone(), env.clone(), cwd.clone()),
            _ => {
                return Err(McpError::Server(
                    "StdioPlugin 收到非 stdio transport 配置".into(),
                ))
            }
        };

        // Resolve ${KEYRING:...} placeholders in env
        let env = super::resolve_env_placeholders(&app, &config.id, &env_raw)?;

        // Windows: `npx`/`npm`/`pnpm` are actually `.cmd` batch scripts; CreateProcess does
        // not resolve extensions via PATHEXT, so `Command::new("npx")` fails with "program not found".
        // Resolve the bare command name to a full path (e.g. `C:\...\npx.cmd`) first; std then
        // wraps execution with `cmd /C` automatically. Falls back to the original command when
        // resolution fails, preserving the native error.
        #[cfg(windows)]
        let program = resolve_command_on_path(&command).unwrap_or_else(|| command.clone());
        #[cfg(not(windows))]
        let program = command.clone();

        // Build the tokio::process::Command
        // Do not inherit the full system env: clear it, then pass the whitelist + user-configured env (user config overrides the whitelist).
        let mut cmd = Command::new(&program);
        cmd.env_clear().envs(inherited_env_whitelist());
        cmd.args(&args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        // Windows: suppress the console window
        #[cfg(windows)]
        {
            #[allow(unused_imports)]
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| McpError::TransportStartup(format!("启动子进程失败: {}", e)))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpError::TransportStartup("无法获取子进程 stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::TransportStartup("无法获取子进程 stdout".into()))?;
        let stderr = child.stderr.take();

        let transport = Box::new(StdioTransportHandle {
            child: tokio::sync::Mutex::new(Some(child)),
            stdin: tokio::sync::Mutex::new(stdin),
            stderr_task: tokio::sync::Mutex::new(None),
        });

        let conn = McpConnection::new(&config.id, "stdio", transport);

        // Start the stdout line-reading loop
        spawn_stdout_reader(
            conn.clone(),
            BufReader::new(stdout),
            app.clone(),
            config.id.clone(),
        );

        // stderr → log events (frontend can subscribe to `mcp-server-stderr` for debugging)
        // MCP-9: 行长上限（一条超长 stderr 行不再无界膨胀事件载荷）
        const STDERR_LINE_LIMIT: usize = 2_000;
        if let Some(stderr) = stderr {
            let app2 = app.clone();
            let sid = config.id.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let capped: String = if line.len() > STDERR_LINE_LIMIT {
                        format!(
                            "{}…[+{} chars]",
                            &line[..line.floor_char_boundary(STDERR_LINE_LIMIT)],
                            line.len() - STDERR_LINE_LIMIT
                        )
                    } else {
                        line
                    };
                    let _ = app2.emit(ev::MCP_SERVER_STDERR, json!({ "id": sid, "line": capped }));
                }
            });
        }

        Ok(conn)
    }

    async fn initialize(&self, conn: &McpConnection) -> McpResult<McpServerInfo> {
        // MCP initialize handshake (upgraded to protocol version 2025-06-18; keeps working with the server's returned version)
        let params = json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {
                "name": "aio",
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        let result = conn
            .request("initialize", Some(params), Duration::from_secs(60))
            .await?;
        // The protocol requires notifications/initialized to be sent after initialize
        conn.notify("notifications/initialized", None).await?;

        // Parse serverInfo
        let info_val = result.get("serverInfo").cloned().unwrap_or(json!({}));
        let name = info_val
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let version = info_val
            .get("version")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        // Parse capabilities
        let capabilities = result
            .get("capabilities")
            .and_then(|v| serde_json::from_value(v.clone()).ok());
        Ok(McpServerInfo {
            name,
            version,
            capabilities,
        })
    }

    async fn list_tools(&self, conn: &McpConnection) -> McpResult<Vec<ToolSpec>> {
        let result = conn
            .request("tools/list", Some(json!({})), Duration::from_secs(60))
            .await?;
        let tools = result
            .get("tools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        // Protocol conversion is centralized in the convert module (single implementation shared by stdio/http)
        Ok(super::convert::tools_to_specs(tools))
    }

    async fn call_tool(
        &self,
        conn: &McpConnection,
        name: &str,
        arguments: Value,
        timeout: Duration,
    ) -> McpResult<ToolResult> {
        let params = json!({ "name": name, "arguments": arguments });
        let result = conn.request("tools/call", Some(params), timeout).await?;
        // MCP returns { content: [...], isError: bool }; parsing is centralized in the convert module
        Ok(super::convert::parse_call_tool_result(&result))
    }

    async fn list_resources(&self, conn: &McpConnection) -> McpResult<Vec<McpResource>> {
        let result = conn
            .request("resources/list", Some(json!({})), Duration::from_secs(60))
            .await?;
        let resources = result
            .get("resources")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(super::convert::parse_resources(resources))
    }

    async fn read_resource(
        &self,
        conn: &McpConnection,
        uri: &str,
    ) -> McpResult<ReadResourceResult> {
        let params = json!({ "uri": uri });
        let result = conn
            .request("resources/read", Some(params), Duration::from_secs(60))
            .await?;
        serde_json::from_value(result).map_err(McpError::from)
    }

    async fn list_prompts(&self, conn: &McpConnection) -> McpResult<Vec<McpPrompt>> {
        let result = conn
            .request("prompts/list", Some(json!({})), Duration::from_secs(60))
            .await?;
        let prompts = result
            .get("prompts")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        Ok(super::convert::parse_prompts(prompts))
    }

    async fn get_prompt(
        &self,
        conn: &McpConnection,
        name: &str,
        arguments: Option<Value>,
    ) -> McpResult<GetPromptResult> {
        let mut params = json!({ "name": name });
        if let Some(args) = arguments {
            params["arguments"] = args;
        }
        let result = conn
            .request("prompts/get", Some(params), Duration::from_secs(60))
            .await?;
        serde_json::from_value(result).map_err(McpError::from)
    }

    async fn stop(&self, conn: McpConnection) -> McpResult<()> {
        conn.close().await
    }
}

struct StdioTransportHandle {
    child: tokio::sync::Mutex<Option<Child>>,
    stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
    /// Keeps the stderr task's JoinHandle (prevents it from being dropped)
    #[allow(dead_code)]
    stderr_task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

#[async_trait]
impl McpTransport for StdioTransportHandle {
    async fn send(&self, payload: &str) -> McpResult<()> {
        let mut stdin = self.stdin.lock().await;
        stdin.write_all(payload.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }

    async fn close(&self) -> McpResult<()> {
        let mut child_guard = self.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            // Termination: attempt kill (tokio's child only supports kill, no graceful shutdown)
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        Ok(())
    }
}

/// Continuously reads stdout in the background; each parsed JSON line is dispatched to McpConnection
fn spawn_stdout_reader(
    conn: McpConnection,
    reader: BufReader<tokio::process::ChildStdout>,
    app: AppHandle,
    server_id: String,
) {
    use tauri::Emitter;
    tokio::spawn(async move {
        let mut lines = reader.lines();
        let mut err_count: u32 = 0;
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Some((method, params)) = conn.dispatch_line(&line) {
                        handle_server_notification(&app, &server_id, &method, &params);
                    }
                }
                Ok(None) => {
                    // EOF
                    let _ = app.emit(
                        ev::MCP_SERVER_STATUS,
                        json!({ "id": server_id, "status": McpStatus::Disconnected }),
                    );
                    break;
                }
                Err(e) => {
                    err_count += 1;
                    if err_count > 10 {
                        let _ = app.emit(
                            ev::MCP_SERVER_STDERR,
                            json!({ "id": server_id, "line": format!("stdout read error: {}", e) }),
                        );
                        break;
                    }
                }
            }
        }
    });
}

/// Consumes notifications pushed proactively by the server.
/// - notifications/tools/list_changed: tool list changed → emit an event so the frontend refreshes its cache
/// - notifications/message (logging): forwarded to the stderr debug channel for display
pub(crate) fn handle_server_notification(
    app: &tauri::AppHandle,
    server_id: &str,
    method: &str,
    params: &serde_json::Value,
) {
    match method {
        "notifications/tools/list_changed" => {
            let _ = app.emit(ev::MCP_TOOLS_CHANGED, json!({ "id": server_id }));
        }
        "notifications/message" => {
            // MCP logging notification: level + data
            let level = params
                .get("level")
                .and_then(|v| v.as_str())
                .unwrap_or("info");
            let data = params
                .get("data")
                .map(|d| d.to_string())
                .unwrap_or_default();
            let _ = app.emit(
                ev::MCP_SERVER_STDERR,
                json!({ "id": server_id, "line": format!("[{level}] {data}") }),
            );
        }
        _ => {
            tracing::debug!("[mcp:{server_id}] 未消费的通知: {method}");
        }
    }
}

/// Windows: resolves a command name to a full executable path.
///
/// `CreateProcess` does not search extensions via `PATHEXT` the way `cmd.exe` does,
/// and `npx`/`npm`/`pnpm` on Windows are actually `.cmd` batch scripts, so
/// `Command::new("npx")` fails with "program not found". This manually walks
/// `PATH` × `PATHEXT` to find the first existing file.
///
/// - Bare command name (e.g. `npx`): walk all PATH directories, appending PATHEXT extensions.
/// - Bare name with a directory (e.g. `C:\nodejs\npx`): only append extensions within that directory.
/// - Already has an extension: returns None; the caller falls back to the original command to preserve the native error.
#[cfg(windows)]
fn resolve_command_on_path(command: &str) -> Option<String> {
    use std::path::{Path, PathBuf};

    let p = Path::new(command);
    if p.extension().is_some() {
        return None;
    }
    let file_name = p.file_name()?;

    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string());
    let exts: Vec<&str> = pathext.split(';').filter(|s| !s.is_empty()).collect();

    // If command contains a directory part, search only that directory; otherwise walk PATH
    let dirs: Vec<PathBuf> = match p.parent() {
        Some(d) if !d.as_os_str().is_empty() => vec![d.to_path_buf()],
        _ => std::env::var("PATH")
            .ok()?
            .split(';')
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .collect(),
    };

    let file_name = file_name.to_string_lossy();
    for dir in dirs {
        for ext in &exts {
            let candidate = dir.join(format!("{}{}", file_name, ext));
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}

/// Utility: derives the placeholder from an existing keyring account (for the UI to call before saving)
#[allow(dead_code)]
pub fn build_keyring_placeholder(server_id: &str, env_key: &str) -> String {
    let account = secure_store::accounts::mcp_server_env(server_id, env_key);
    format!("${{KEYRING:{}}}", account)
}
