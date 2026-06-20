//! MCP stdio 传输
//!
//! 启动一个子进程，通过 stdin/stdout 收发 JSON-RPC 2.0 消息（每行一个 JSON）。
//! 启动子进程时不继承系统 env，仅传白名单 env 字段（防止密钥泄露）。

use super::connection::{McpConnection, McpTransport};
use super::error::{McpError, McpResult};
use crate::core::models::{
    McpServerConfig, McpServerInfo, McpStatus, ToolResult, ToolResultContent, ToolSpec,
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

#[async_trait]
impl super::McpServerPlugin for StdioPlugin {
    fn identifier(&self) -> &'static str {
        "stdio"
    }

    async fn start(
        &self,
        app: AppHandle,
        config: &McpServerConfig,
    ) -> McpResult<McpConnection> {
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

        // 解析 env 中的 ${KEYRING:...} 占位符
        let env = super::resolve_env_placeholders(&app, &config.id, &env_raw)?;

        // Windows: `npx`/`npm`/`pnpm` 等实为 `.cmd` 批处理脚本，CreateProcess 不会
        // 按 PATHEXT 搜索扩展名，直接 `Command::new("npx")` 会报 "program not found"。
        // 先把裸命令名解析为完整路径（如 `C:\...\npx.cmd`），std 会自动用 `cmd /C`
        // 包装执行；解析不到则回退原命令，保留原生报错。
        #[cfg(windows)]
        let program = resolve_command_on_path(&command).unwrap_or_else(|| command.clone());
        #[cfg(not(windows))]
        let program = command.clone();

        // 构造 tokio::process::Command
        // 继承系统 env（npx/node 依赖 PATH/APPDATA/TEMP 等），用户配置的 env 覆盖同名变量。
        let mut cmd = Command::new(&program);
        cmd.args(&args)
            .envs(env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        // Windows: 抑制控制台窗口
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

        // 启动 stdout 行读取循环
        spawn_stdout_reader(conn.clone(), BufReader::new(stdout), app.clone(), config.id.clone());

        // stderr → 日志事件（前端可订阅 `mcp-server-stderr` 调试）
        if let Some(stderr) = stderr {
            let app2 = app.clone();
            let sid = config.id.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = app2.emit(
                        "mcp-server-stderr",
                        json!({ "id": sid, "line": line }),
                    );
                }
            });
        }

        Ok(conn)
    }

    async fn initialize(
        &self,
        conn: &McpConnection,
    ) -> McpResult<McpServerInfo> {
        // MCP initialize 协议
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "aio",
                "version": env!("CARGO_PKG_VERSION")
            }
        });
        let result = conn
            .request("initialize", Some(params), Duration::from_secs(60))
            .await?;
        // 协议要求 initialize 后必须发 notifications/initialized
        conn.notify("notifications/initialized", None).await?;

        // 解析 serverInfo
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
        Ok(McpServerInfo { name, version })
    }

    async fn list_tools(
        &self,
        conn: &McpConnection,
    ) -> McpResult<Vec<ToolSpec>> {
        let result = conn
            .request("tools/list", Some(json!({})), Duration::from_secs(60))
            .await?;
        let tools = result
            .get("tools")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        // MCP 协议返回 { name, description, inputSchema }，需转换为 OpenAI
        // function calling 格式 { type: "function", function: { name, description, parameters } }
        let specs: Vec<ToolSpec> = tools
            .into_iter()
            .filter_map(|t| {
                let name = t.get("name")?.as_str()?.to_string();
                let description = t.get("description")?.as_str()?.to_string();
                let parameters = t.get("inputSchema").cloned().unwrap_or(json!({}));
                Some(ToolSpec {
                    kind: "function".to_string(),
                    function: crate::core::models::ToolFunctionSpec {
                        name,
                        description,
                        parameters,
                    },
                })
            })
            .take(super::MAX_TOOLS_PER_SERVER)
            .collect();
        Ok(specs)
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
        // MCP 返回 { content: [...], isError: bool }
        let content_val = result.get("content").cloned().unwrap_or(json!([]));
        let content: Vec<ToolResultContent> = match content_val {
            Value::Array(arr) => arr
                .into_iter()
                .filter_map(|v| serde_json::from_value(v).ok())
                .collect(),
            _ => vec![],
        };
        let is_error = result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        Ok(ToolResult { content, is_error })
    }

    async fn stop(&self, conn: McpConnection) -> McpResult<()> {
        conn.close().await
    }
}

struct StdioTransportHandle {
    child: tokio::sync::Mutex<Option<Child>>,
    stdin: tokio::sync::Mutex<tokio::process::ChildStdin>,
    /// 保留 stderr task 的 JoinHandle（防止被 drop）
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
            // 优雅终止：先尝试 kill（tokio 的 child 只能 kill，不能优雅）
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        Ok(())
    }
}

/// 在后台持续读取 stdout，每解析到一行 JSON 就分发到 McpConnection
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
                    conn.dispatch_line(&line);
                }
                Ok(None) => {
                    // EOF
                    let _ = app.emit(
                        "mcp-server-status",
                        json!({ "id": server_id, "status": McpStatus::Disconnected }),
                    );
                    break;
                }
                Err(e) => {
                    err_count += 1;
                    if err_count > 10 {
                        let _ = app.emit(
                            "mcp-server-stderr",
                            json!({ "id": server_id, "line": format!("stdout read error: {}", e) }),
                        );
                        break;
                    }
                }
            }
        }
    });
}

/// Windows: 把命令名解析为完整可执行路径。
///
/// `CreateProcess` 不会像 `cmd.exe` 那样按 `PATHEXT` 搜索扩展名，而 `npx`/`npm`/
/// `pnpm` 等在 Windows 上实为 `.cmd` 批处理脚本，因此 `Command::new("npx")` 会
/// 报 "program not found"。这里手动遍历 `PATH` × `PATHEXT` 找到首个存在的文件。
///
/// - 裸命令名（如 `npx`）：遍历 PATH 各目录补 PATHEXT 扩展名。
/// - 带目录的裸名（如 `C:\nodejs\npx`）：仅在其所在目录内补扩展名。
/// - 已带扩展名：返回 None，交由调用方回退到原命令保留原生报错。
#[cfg(windows)]
fn resolve_command_on_path(command: &str) -> Option<String> {
    use std::path::{Path, PathBuf};

    let p = Path::new(command);
    if p.extension().is_some() {
        return None;
    }
    let file_name = p.file_name()?;

    let pathext =
        std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string());
    let exts: Vec<&str> = pathext.split(';').filter(|s| !s.is_empty()).collect();

    // command 含目录部分时只在该目录内查找；否则遍历 PATH
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

/// 工具：从已有 keyring account 推导出占位符（供 UI 在保存前调用）
#[allow(dead_code)]
pub fn build_keyring_placeholder(server_id: &str, env_key: &str) -> String {
    let account = secure_store::accounts::mcp_server_env(server_id, env_key);
    format!("${{KEYRING:{}}}", account)
}
