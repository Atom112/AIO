//! LspClient - manages the connection to a single language server
//!
//! Encapsulates the LSP protocol lifecycle: initialize / shutdown, document
//! synchronization, and diagnostics notification handling.
//!
//! Design modeled on `McpConnection` in `plugins/mcp/connection.rs`.

use super::error::LspResult;
use super::transport::{encode_message, read_frame};
use crate::ierr;
use crate::utils::jsonrpc::{PendingMap, WaitError};
use dashmap::DashMap;
use lsp_types::PublishDiagnosticsParams;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

#[allow(unused_imports)]
#[cfg(windows)]
use std::os::windows::process::CommandExt;

// ====== LspClient ======

/// Connection abstraction for a single language server
#[derive(Clone)]
pub struct LspClient {
    /// Language identifier (e.g. "typescript", "rust")
    pub language_id: String,
    /// Project root path
    pub project_root: PathBuf,
    pub(crate) inner: std::sync::Arc<LspClientInner>,
}

pub(crate) struct LspClientInner {
    /// JSON-RPC pending request table (utils::jsonrpc::PendingMap shared with the MCP client)
    pub pending: PendingMap<Value>,
    /// Child process handle
    pub child: Mutex<Option<Child>>,
    /// stdin writer
    pub stdin: Mutex<Option<ChildStdin>>,
    /// Currently accumulated diagnostics: uri -> diagnostics
    pub diagnostics: DashMap<String, Vec<lsp_types::Diagnostic>>,
}

impl LspClient {
    /// Spawn the language server child process and send the initialize handshake
    pub async fn start(
        language_id: impl Into<String>,
        project_root: PathBuf,
        command: &str,
        args: &[String],
        app: AppHandle,
    ) -> LspResult<Self> {
        let language_id = language_id.into();
        let root_uri = path_to_uri(&project_root)?;

        // Windows: resolve the command on PATH (mirrors resolve_command_on_path in MCP stdio)
        #[cfg(windows)]
        let program = resolve_lsp_command(command).unwrap_or_else(|| command.to_string());
        #[cfg(not(windows))]
        let program = command.to_string();

        let mut cmd = Command::new(&program);
        cmd.args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);

        // Windows: hide the console window
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().map_err(|e| {
            super::error::LspError::TransportStartup(ierr!(
                "启动语言服务器 {} 失败: {}",
                "Failed to start language server {}: {}",
                command,
                e
            ))
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            super::error::LspError::TransportStartup(ierr!(
                "无法获取子进程 stdin",
                "Failed to acquire the child process stdin"
            ))
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            super::error::LspError::TransportStartup(ierr!(
                "无法获取子进程 stdout",
                "Failed to acquire the child process stdout"
            ))
        })?;
        let stderr = child.stderr.take();

        let client = LspClient {
            language_id: language_id.clone(),
            project_root: project_root.clone(),
            inner: std::sync::Arc::new(LspClientInner {
                pending: PendingMap::new(),
                child: Mutex::new(Some(child)),
                stdin: Mutex::new(Some(stdin)),
                diagnostics: DashMap::new(),
            }),
        };

        // Start the stdout reader loop
        spawn_stdout_reader(
            client.clone(),
            BufReader::new(stdout),
            app.clone(),
            language_id.clone(),
        );

        // stderr -> log events
        if let Some(stderr) = stderr {
            let app2 = app.clone();
            let lid = language_id.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = app2.emit(
                        "lsp-server-stderr",
                        serde_json::json!({ "languageId": lid, "line": line }),
                    );
                }
            });
        }

        // Send the initialize request
        client.initialize(&root_uri).await?;

        Ok(client)
    }

    /// Send the initialize request + initialized notification
    async fn initialize(&self, root_uri: &str) -> LspResult<()> {
        let params = serde_json::json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "rootPath": self.project_root.to_string_lossy(),
            "capabilities": {
                "textDocument": {
                    "publishDiagnostics": {
                        "relatedInformation": true,
                        "tagSupport": { "valueSet": [1, 2] }
                    }
                },
            },
            "workspaceFolders": [{
                "uri": root_uri,
                "name": self.project_root.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "root".to_string())
            }],
        });

        let _result = self
            .request("initialize", Some(params), Duration::from_secs(30))
            .await?;

        // Send the initialized notification
        self.notify("initialized", Some(serde_json::json!({})))
            .await?;

        Ok(())
    }

    /// Send the shutdown request and wait for exit
    pub async fn shutdown(&self) -> LspResult<()> {
        // Send the shutdown request
        let _ = self.request("shutdown", None, Duration::from_secs(5)).await;
        // Send the exit notification
        let _ = self.notify("exit", None).await;

        // Wait for the child process to exit
        let mut child_guard = self.inner.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }

        Ok(())
    }

    /// Get diagnostics for a given file
    pub fn get_diagnostics(
        &self,
        file_path: Option<&str>,
    ) -> Vec<(String, Vec<lsp_types::Diagnostic>)> {
        let mut result = Vec::new();

        if let Some(fp) = file_path {
            if let Ok(uri) = file_path_to_uri(&self.project_root, fp) {
                if let Some(diags) = self.inner.diagnostics.get(&uri) {
                    result.push((uri, diags.clone()));
                }
            }
        } else {
            for entry in self.inner.diagnostics.iter() {
                result.push((entry.key().clone(), entry.value().clone()));
            }
        }

        result
    }

    /// Clear diagnostics for a given file or for all files
    #[allow(dead_code)]
    pub fn clear_diagnostics(&self, file_path: Option<&str>) {
        if let Some(fp) = file_path {
            if let Ok(uri) = file_path_to_uri(&self.project_root, fp) {
                self.inner.diagnostics.remove(&uri);
            }
        } else {
            self.inner.diagnostics.clear();
        }
    }

    /// Send a JSON-RPC request and wait for the response
    pub(crate) async fn request(
        &self,
        method: &str,
        params: Option<Value>,
        timeout_dur: Duration,
    ) -> LspResult<Value> {
        let (id, rx) = self.inner.pending.register();
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let payload = serde_json::to_string(&req)?;
        let frame = encode_message(&payload);

        // Send
        {
            let mut stdin_guard = self.inner.stdin.lock().await;
            if let Some(ref mut stdin) = *stdin_guard {
                stdin.write_all(frame.as_bytes()).await.map_err(|e| {
                    super::error::LspError::Transport(format!("写入 stdin 失败: {}", e))
                })?;
                stdin.flush().await.map_err(|e| {
                    super::error::LspError::Transport(format!("flush stdin 失败: {}", e))
                })?;
            } else {
                self.inner.pending.cancel(id);
                return Err(super::error::LspError::Transport("stdin 已关闭".into()));
            }
        }

        // Wait for the response
        let resp = match self.inner.pending.wait(id, rx, timeout_dur).await {
            Ok(v) => v,
            Err(WaitError::Canceled) => {
                return Err(super::error::LspError::Protocol(
                    "oneshot canceled (sender dropped)".into(),
                ))
            }
            Err(WaitError::Timeout(d)) => {
                return Err(super::error::LspError::Timeout(d));
            }
        };

        // Check for errors
        if let Some(err) = resp.get("error") {
            let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
            let message = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error")
                .to_string();
            return Err(super::error::LspError::Remote {
                code: code as i32,
                message,
            });
        }

        Ok(resp.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Send a JSON-RPC notification (no id, no response awaited)
    pub(crate) async fn notify(&self, method: &str, params: Option<Value>) -> LspResult<()> {
        let n = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let payload = serde_json::to_string(&n)?;
        let frame = encode_message(&payload);

        let mut stdin_guard = self.inner.stdin.lock().await;
        if let Some(ref mut stdin) = *stdin_guard {
            stdin.write_all(frame.as_bytes()).await.map_err(|e| {
                super::error::LspError::Transport(format!("写入 stdin 失败: {}", e))
            })?;
            stdin.flush().await.map_err(|e| {
                super::error::LspError::Transport(format!("flush stdin 失败: {}", e))
            })?;
        }

        Ok(())
    }

    /// Handle a single message read from stdout
    pub(crate) fn dispatch_message(&self, line: &str) {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return,
        };

        // Has id but no method -> it is a response
        if v.get("id").is_some() && v.get("method").is_none() {
            if let Some(id) = v.get("id").and_then(|i| i.as_u64()) {
                if let Some(tx) = self.inner.pending.take(id) {
                    let _ = tx.send(v);
                }
            }
            return;
        }

        // Handle the notification
        if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
            if method == "textDocument/publishDiagnostics" {
                if let Some(params) = v.get("params") {
                    if let Ok(p) =
                        serde_json::from_value::<PublishDiagnosticsParams>(params.clone())
                    {
                        let uri = p.uri.to_string();
                        if p.diagnostics.is_empty() {
                            self.inner.diagnostics.remove(&uri);
                        } else {
                            self.inner.diagnostics.insert(uri, p.diagnostics);
                        }
                    }
                }
            }
        }
    }
}

// ====== Helper functions ======

/// Filesystem path -> file:// URI
fn path_to_uri(path: &Path) -> LspResult<String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|e| super::error::LspError::Protocol(format!("canonicalize 路径失败: {}", e)))?;
    let url = url::Url::from_file_path(&canonical).map_err(|_| {
        super::error::LspError::Protocol(format!("路径转 URI 失败: {}", canonical.display()))
    })?;
    Ok(url.to_string())
}

/// Relative file path -> file:// URI
fn file_path_to_uri(project_root: &Path, file_path: &str) -> LspResult<String> {
    let full = project_root.join(file_path);
    path_to_uri(&full)
}

/// Continuously read stdout in the background, dispatching each parsed LSP frame to the LspClient
fn spawn_stdout_reader(
    client: LspClient,
    mut reader: BufReader<ChildStdout>,
    app: AppHandle,
    language_id: String,
) {
    tokio::spawn(async move {
        let mut err_count: u32 = 0;
        loop {
            match read_frame(&mut reader).await {
                Ok(Some(body)) => {
                    client.dispatch_message(&body);
                    // Check for a publishDiagnostics notification and forward it to the frontend event
                    if let Ok(v) = serde_json::from_str::<Value>(&body) {
                        if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
                            if method == "textDocument/publishDiagnostics" {
                                if let Some(params) = v.get("params") {
                                    let _ = app.emit(
                                        "lsp-diagnostics-updated",
                                        serde_json::json!({
                                            "languageId": language_id,
                                            "params": params,
                                        }),
                                    );
                                }
                            }
                        }
                    }
                }
                Ok(None) => {
                    let _ = app.emit(
                        "lsp-server-status",
                        serde_json::json!({
                            "languageId": language_id,
                            "status": "stopped"
                        }),
                    );
                    break;
                }
                Err(e) => {
                    err_count += 1;
                    if err_count > 10 {
                        let _ = app.emit(
                            "lsp-server-stderr",
                            serde_json::json!({
                                "languageId": language_id,
                                "line": format!("stdout read error: {}", e),
                            }),
                        );
                        break;
                    }
                }
            }
        }
    });
}

// ====== Windows command resolution ======

#[cfg(windows)]
fn resolve_lsp_command(command: &str) -> Option<String> {
    use std::path::{Path, PathBuf};

    let p = Path::new(command);
    if p.extension().is_some() {
        return None;
    }
    let file_name = p.file_name()?;

    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string());
    let exts: Vec<&str> = pathext.split(';').filter(|s| !s.is_empty()).collect();

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
