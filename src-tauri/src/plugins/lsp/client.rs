//! LspClient - manages the connection to a single language server
//!
//! Encapsulates the LSP protocol lifecycle: initialize / shutdown, document
//! synchronization, and diagnostics notification handling.
//!
//! Design modeled on `McpConnection` in `plugins/mcp/connection.rs`.

use super::error::LspResult;
use super::transport::{encode_message, read_frame};
use crate::core::events as ev;
use crate::ierr;
use crate::utils::jsonrpc::{PendingMap, WaitError};
use dashmap::DashMap;
use lsp_types::PublishDiagnosticsParams;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

#[allow(unused_imports)]
#[cfg(windows)]
use std::os::windows::process::CommandExt;

// ====== LspClient ======

/// Event sink: `(event_name, payload)`. The LSP client stays decoupled from the
/// Tauri runtime — production callers wrap an `AppHandle::emit` into this shape
/// (see `commands/lsp.rs` / `utils/lsp_agent_tools.rs`), tests pass a no-op.
pub type LspEventSink = std::sync::Arc<dyn Fn(&'static str, Value) + Send + Sync>;

/// Wraps an AppHandle's emit into an [`LspEventSink`].
pub fn app_event_sink<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> LspEventSink {
    use tauri::Emitter as _;
    let app = app.clone();
    std::sync::Arc::new(move |event, payload| {
        let _ = app.emit(event, payload);
    })
}

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
    /// Latest synced version per open document: uri -> version (didOpen on first
    /// sync, didChange afterwards — see `LspClient::sync_document`)
    pub open_docs: DashMap<String, u32>,
}

impl LspClient {
    /// Spawn the language server child process and send the initialize handshake.
    /// `sink` receives the client's events (status / stderr / diagnostics); pass
    /// `app_event_sink(&app)` in production.
    pub async fn start(
        language_id: impl Into<String>,
        project_root: PathBuf,
        command: &str,
        args: &[String],
        sink: LspEventSink,
    ) -> LspResult<Self> {
        let language_id = language_id.into();
        let root_uri = path_to_uri(&project_root)?;

        // SEC-06: canonical PATH-only resolution (also handles the Windows PATHEXT
        // walk); falls back to the configured command so the OS reports the native error.
        let program = crate::utils::exec_resolve::resolve_or_keep(command);

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
                open_docs: DashMap::new(),
            }),
        };

        // Start the stdout reader loop
        spawn_stdout_reader(
            client.clone(),
            BufReader::new(stdout),
            sink.clone(),
            language_id.clone(),
        );

        // stderr -> log events
        if let Some(stderr) = stderr {
            let sink2 = sink.clone();
            let lid = language_id.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncBufReadExt;
                let mut lines = tokio::io::BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    sink2(
                        ev::LSP_SERVER_STDERR,
                        serde_json::json!({ "languageId": lid, "line": line }),
                    );
                }
            });
        }

        // Send the initialize request. On failure, shut the child down explicitly:
        // the stdout reader task holds an Arc clone, so kill_on_drop never fires on
        // its own and an early `?` return would leak the language-server process.
        if let Err(e) = client.initialize(&root_uri).await {
            let _ = client.shutdown().await;
            return Err(e);
        }

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

    /// Syncs a document's current content into the server: `didOpen` the first
    /// time a URI is synced, `didChange` (full-text replacement) afterwards.
    ///
    /// Why: nothing in AIO ever opens documents (the frontend is not an editor,
    /// and the agent tools query files directly), so a freshly started server
    /// has usually not analyzed the target file yet and answers symbol /
    /// hover / diagnostics queries with empty results. didOpen makes the server
    /// analyze the file (and publish its diagnostics) before the query runs.
    pub async fn sync_document(&self, uri: &str, language_id: &str, text: &str) -> LspResult<()> {
        // Advance the version; remember whether this is the first sync.
        // (The entry guard must drop before awaiting notify below.)
        let (first_sync, version) = {
            use dashmap::mapref::entry::Entry;
            match self.inner.open_docs.entry(uri.to_string()) {
                Entry::Occupied(mut occ) => {
                    *occ.get_mut() += 1;
                    (false, *occ.get())
                }
                Entry::Vacant(vac) => {
                    vac.insert(1);
                    (true, 1u32)
                }
            }
        };
        let (method, params) = if first_sync {
            (
                "textDocument/didOpen",
                serde_json::json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": language_id,
                        "text": text,
                        "version": version,
                    }
                }),
            )
        } else {
            (
                "textDocument/didChange",
                serde_json::json!({
                    "textDocument": { "uri": uri, "version": version },
                    "contentChanges": [ { "text": text } ],
                }),
            )
        };
        self.notify(method, Some(params)).await
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
    sink: LspEventSink,
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
                                    sink(
                                        ev::LSP_DIAGNOSTICS_UPDATED,
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
                    sink(
                        ev::LSP_SERVER_STATUS,
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
                        sink(
                            ev::LSP_SERVER_STDERR,
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
// (removed: superseded by utils::exec_resolve, the repo-wide PATH-only resolver)

#[cfg(test)]
mod sync_tests {
    use super::*;
    use std::time::Duration;

    /// Fake stdio LSP server (node): completes the initialize handshake, records
    /// didOpen/didChange per URI, and answers `documentSymbol` with two symbols
    /// ONLY for URIs it has seen a document sync for — mirroring how real servers
    /// (pyright et al.) answer empty for files they have not analyzed.
    const FAKE_SERVER: &str = r#"
let buf = '';
const opened = {};
process.stdin.setEncoding('utf8');
function send(obj) {
  const body = JSON.stringify(obj);
  process.stdout.write('Content-Length: ' + Buffer.byteLength(body) + '\r\n\r\n' + body);
}
process.stdin.on('data', (d) => {
  buf += d;
  for (;;) {
    const idx = buf.indexOf('\r\n\r\n');
    if (idx < 0) break;
    const header = buf.slice(0, idx);
    const m = /Content-Length:\s*(\d+)/i.exec(header);
    if (!m) { buf = buf.slice(idx + 4); continue; }
    const len = parseInt(m[1], 10);
    if (buf.length < idx + 4 + len) break;
    const body = buf.slice(idx + 4, idx + 4 + len);
    buf = buf.slice(idx + 4 + len);
    let msg;
    try { msg = JSON.parse(body); } catch { continue; }
    if (msg.method === 'initialize') {
      send({ jsonrpc: '2.0', id: msg.id, result: { capabilities: { documentSymbolProvider: true } } });
    } else if (msg.method === 'shutdown') {
      send({ jsonrpc: '2.0', id: msg.id, result: null });
    } else if (msg.method === 'exit') {
      process.exit(0);
    } else if (msg.method === 'textDocument/didOpen' || msg.method === 'textDocument/didChange') {
      opened[msg.params.textDocument.uri] = msg.params.textDocument;
    } else if (msg.id !== undefined && msg.method) {
      if (msg.method === 'textDocument/documentSymbol') {
        const uri = msg.params.textDocument.uri;
        const result = opened[uri] ? [
          { name: 'greet', kind: 12, range: { start: { line: 0, character: 0 }, end: { line: 2, character: 0 } } },
          { name: 'add', kind: 12, range: { start: { line: 3, character: 0 }, end: { line: 5, character: 0 } } },
        ] : [];
        send({ jsonrpc: '2.0', id: msg.id, result });
      } else {
        send({ jsonrpc: '2.0', id: msg.id, result: null });
      }
    }
  }
});
"#;

    /// Regression for the harness-test finding: queries racing the analyzer.
    /// Before a `sync_document` (didOpen) the server answers documentSymbol with
    /// an empty array; after the sync the symbols are visible, and a second
    /// sync degrades to didChange without breaking the query flow.
    /// Skips silently when node is unavailable (PATH-only resolution).
    #[tokio::test]
    async fn sync_document_makes_symbols_visible() {
        if crate::utils::exec_resolve::resolve_executable("node").is_none() {
            return; // node absent on this machine — nothing to test against
        }

        let root = std::env::temp_dir().join("aio_lsp_sync_test");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("创建临时目录失败");
        std::fs::write(root.join("sample.py"), "def greet():\n    return 1\n")
            .expect("写入样本失败");
        let script = std::env::temp_dir().join("aio_fake_lsp_server.js");
        std::fs::write(&script, FAKE_SERVER).expect("写入假服务器脚本失败");

        let sink: LspEventSink = std::sync::Arc::new(|_event: &'static str, _payload: Value| {});
        let client = LspClient::start(
            "python",
            root.clone(),
            "node",
            &[script.to_string_lossy().to_string()],
            sink,
        )
        .await
        .expect("假语言服务器启动失败");

        let uri = url::Url::from_file_path(root.join("sample.py"))
            .expect("路径转 URI 失败")
            .to_string();
        let doc = serde_json::json!({ "textDocument": { "uri": uri } });

        // 1) Before any sync: the file was never opened → empty answer
        let r = client
            .request(
                "textDocument/documentSymbol",
                Some(doc.clone()),
                Duration::from_secs(5),
            )
            .await
            .expect("请求失败");
        assert_eq!(
            r.as_array().map(Vec::len),
            Some(0),
            "未同步的文件应返回空符号"
        );

        // 2) First sync = didOpen → symbols become visible
        client
            .sync_document(&uri, "python", "def greet():\n    return 1\n")
            .await
            .expect("didOpen 失败");
        let r = client
            .request(
                "textDocument/documentSymbol",
                Some(doc.clone()),
                Duration::from_secs(5),
            )
            .await
            .expect("请求失败");
        assert_eq!(r.as_array().map(Vec::len), Some(2), "didOpen 后应返回符号");

        // 3) Second sync = didChange (version bump) → still works
        client
            .sync_document(&uri, "python", "def greet():\n    return 2\n")
            .await
            .expect("didChange 失败");
        let r = client
            .request(
                "textDocument/documentSymbol",
                Some(doc),
                Duration::from_secs(5),
            )
            .await
            .expect("请求失败");
        assert_eq!(
            r.as_array().map(Vec::len),
            Some(2),
            "didChange 后应仍返回符号"
        );

        let _ = client.shutdown().await;
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_file(&script);
    }
}
