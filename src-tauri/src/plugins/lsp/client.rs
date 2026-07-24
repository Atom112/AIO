//! LspClient — 管理单个语言服务器的连接
//!
//! 封装 LSP 协议的生命周期：initialize / shutdown，文档同步，
//! 以及诊断通知的处理。
//!
//! 参考 `plugins/mcp/connection.rs` 的 `McpConnection` 设计模式。

use super::error::LspResult;
use super::transport::{encode_message, read_frame};
use dashmap::DashMap;
use lsp_types::PublishDiagnosticsParams;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::str::FromStr;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::oneshot;
use tokio::sync::Mutex;

#[allow(unused_imports)]
#[cfg(windows)]
use std::os::windows::process::CommandExt;

// ====== LspClient ======

/// 单个语言服务器的连接抽象
#[derive(Clone)]
pub struct LspClient {
    /// 语言标识符（如 "typescript", "rust"）
    pub language_id: String,
    /// 项目根路径
    pub project_root: PathBuf,
    pub(crate) inner: std::sync::Arc<LspClientInner>,
}

pub(crate) struct LspClientInner {
    /// JSON-RPC 请求 id 生成器
    pub next_id: AtomicU64,
    /// 等待响应的 pending 请求映射 (id → oneshot sender)
    pub pending: DashMap<u64, oneshot::Sender<Value>>,
    /// 子进程句柄
    pub child: Mutex<Option<Child>>,
    /// stdin 写入端
    pub stdin: Mutex<Option<ChildStdin>>,
    /// 当前累积的诊断：uri → diagnostics
    pub diagnostics: DashMap<String, Vec<lsp_types::Diagnostic>>,
    /// 根 URI
    pub root_uri: String,
}

impl LspClient {
    /// 启动语言服务器子进程，发送 initialize 握手
    pub async fn start(
        language_id: impl Into<String>,
        project_root: PathBuf,
        command: &str,
        args: &[String],
        app: AppHandle,
    ) -> LspResult<Self> {
        let language_id = language_id.into();
        let root_uri = path_to_uri(&project_root)?;

        // Windows: 解析命令路径（参考 MCP stdio 的 resolve_command_on_path）
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

        // Windows: 隐藏控制台窗口
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = cmd.spawn().map_err(|e| {
            super::error::LspError::TransportStartup(format!(
                "启动语言服务器 {} 失败: {}",
                command, e
            ))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| super::error::LspError::TransportStartup("无法获取子进程 stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| super::error::LspError::TransportStartup("无法获取子进程 stdout".into()))?;
        let stderr = child.stderr.take();

        let client = LspClient {
            language_id: language_id.clone(),
            project_root: project_root.clone(),
            inner: std::sync::Arc::new(LspClientInner {
                next_id: AtomicU64::new(1),
                pending: DashMap::new(),
                child: Mutex::new(Some(child)),
                stdin: Mutex::new(Some(stdin)),
                diagnostics: DashMap::new(),
                root_uri: root_uri.clone(),
            }),
        };

        // 启动 stdout 读取循环
        spawn_stdout_reader(
            client.clone(),
            BufReader::new(stdout),
            app.clone(),
            language_id.clone(),
        );

        // stderr → 日志事件
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

        // 发送 initialize 请求
        client.initialize(&root_uri).await?;

        Ok(client)
    }

    /// 发送 initialize 请求 + initialized 通知
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

        // 发送 initialized 通知
        self.notify("initialized", Some(serde_json::json!({})))
            .await?;

        Ok(())
    }

    /// 发送 textDocument/didOpen 通知，触发诊断
    pub async fn text_document_did_open(
        &self,
        file_path: &str,
        _language_id: &str,
        text: &str,
    ) -> LspResult<()> {
        let uri = file_path_to_uri(&self.project_root, file_path)?;
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri,
                "languageId": _language_id,
                "version": 1,
                "text": text,
            }
        });
        self.notify("textDocument/didOpen", Some(params)).await
    }

    /// 发送 textDocument/didChange 通知
    #[allow(dead_code)]
    pub async fn text_document_did_change(
        &self,
        file_path: &str,
        text: &str,
        version: i32,
    ) -> LspResult<()> {
        let uri = file_path_to_uri(&self.project_root, file_path)?;
        let params = serde_json::json!({
            "textDocument": {
                "uri": uri,
                "version": version,
            },
            "contentChanges": [
                { "text": text }
            ]
        });
        self.notify("textDocument/didChange", Some(params)).await
    }

    /// 发送 textDocument/didClose 通知
    #[allow(dead_code)]
    pub async fn text_document_did_close(&self, file_path: &str) -> LspResult<()> {
        let uri = file_path_to_uri(&self.project_root, file_path)?;
        let params = serde_json::json!({
            "textDocument": { "uri": uri }
        });
        self.notify("textDocument/didClose", Some(params)).await
    }

    /// 发送 shutdown 请求并等待退出
    pub async fn shutdown(&self) -> LspResult<()> {
        // 发送 shutdown 请求
        let _ = self
            .request("shutdown", None, Duration::from_secs(5))
            .await;
        // 发送 exit 通知
        let _ = self.notify("exit", None).await;

        // 等待子进程退出
        let mut child_guard = self.inner.child.lock().await;
        if let Some(mut child) = child_guard.take() {
            let _ = child.kill().await;
            let _ = child.wait().await;
        }

        Ok(())
    }

    /// 获取指定文件的诊断
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

    /// 清除指定文件或所有文件的诊断
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

    /// 发送 JSON-RPC request 并等待响应
    pub(crate) async fn request(
        &self,
        method: &str,
        params: Option<Value>,
        timeout_dur: Duration,
    ) -> LspResult<Value> {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let (tx, rx) = oneshot::channel();
        self.inner.pending.insert(id, tx);

        let payload = serde_json::to_string(&req)?;
        let frame = encode_message(&payload);

        // 发送
        {
            let mut stdin_guard = self.inner.stdin.lock().await;
            if let Some(ref mut stdin) = *stdin_guard {
                stdin
                    .write_all(frame.as_bytes())
                    .await
                    .map_err(|e| super::error::LspError::Transport(format!("写入 stdin 失败: {}", e)))?;
                stdin
                    .flush()
                    .await
                    .map_err(|e| super::error::LspError::Transport(format!("flush stdin 失败: {}", e)))?;
            } else {
                self.inner.pending.remove(&id);
                return Err(super::error::LspError::Transport("stdin 已关闭".into()));
            }
        }

        // 等待响应
        let resp = match tokio::time::timeout(timeout_dur, rx).await {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => {
                self.inner.pending.remove(&id);
                return Err(super::error::LspError::Protocol(
                    "oneshot canceled (sender dropped)".into(),
                ));
            }
            Err(_) => {
                self.inner.pending.remove(&id);
                return Err(super::error::LspError::Timeout(timeout_dur));
            }
        };

        // 检查错误
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

    /// 发送 JSON-RPC notification（无 id，不等响应）
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
            stdin
                .write_all(frame.as_bytes())
                .await
                .map_err(|e| super::error::LspError::Transport(format!("写入 stdin 失败: {}", e)))?;
            stdin
                .flush()
                .await
                .map_err(|e| super::error::LspError::Transport(format!("flush stdin 失败: {}", e)))?;
        }

        Ok(())
    }

    // ====== Agent 工具方法 ======

    /// Go-to-definition: 返回定义位置的 Location 或 null
    pub async fn goto_definition(
        &self,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> LspResult<Option<lsp_types::Location>> {
        let uri = file_path_to_uri(&self.project_root, file_path)?;
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        });
        let result = self
            .request("textDocument/definition", Some(params), std::time::Duration::from_secs(10))
            .await?;
        if result.is_null() {
            return Ok(None);
        }
        // Try Vec<Location> first (returns first definition), then single Location
        if let Ok(mut locations) = serde_json::from_value::<Vec<lsp_types::Location>>(result.clone()) {
            Ok(if locations.is_empty() { None } else { Some(locations.remove(0)) })
        } else if let Ok(loc) = serde_json::from_value::<lsp_types::Location>(result) {
            Ok(Some(loc))
        } else {
            Ok(None)
        }
    }

    /// Find all references: 返回所有引用位置
    pub async fn find_references(
        &self,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> LspResult<Vec<lsp_types::Location>> {
        let uri = file_path_to_uri(&self.project_root, file_path)?;
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character },
            "context": { "includeDeclaration": true }
        });
        let result = self
            .request("textDocument/references", Some(params), std::time::Duration::from_secs(10))
            .await?;
        if result.is_null() {
            return Ok(Vec::new());
        }
        serde_json::from_value::<Vec<lsp_types::Location>>(result)
            .map_err(|e| super::error::LspError::Protocol(format!("解析 references 响应失败: {}", e)))
    }

    /// Hover info: 返回 hover 内容的 markdown 字符串
    pub async fn hover(
        &self,
        file_path: &str,
        line: u32,
        character: u32,
    ) -> LspResult<Option<String>> {
        let uri = file_path_to_uri(&self.project_root, file_path)?;
        let params = serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        });
        let result = self
            .request("textDocument/hover", Some(params), std::time::Duration::from_secs(10))
            .await?;
        if result.is_null() {
            return Ok(None);
        }
        let hover: lsp_types::Hover = serde_json::from_value(result)
            .map_err(|e| super::error::LspError::Protocol(format!("解析 hover 响应失败: {}", e)))?;
        let text = match hover.contents {
            lsp_types::HoverContents::Scalar(s) => match s {
                lsp_types::MarkedString::String(s) => s,
                lsp_types::MarkedString::LanguageString(ls) => ls.value,
            },
            lsp_types::HoverContents::Markup(mc) => mc.value,
            lsp_types::HoverContents::Array(arr) => arr
                .iter()
                .map(|s| match s {
                    lsp_types::MarkedString::String(s) => s.clone(),
                    lsp_types::MarkedString::LanguageString(ls) => ls.value.clone(),
                })
                .collect::<Vec<_>>()
                .join("\n"),
        };
        if text.is_empty() {
            Ok(None)
        } else {
            Ok(Some(text))
        }
    }

    /// Document symbols: 返回文件中的所有符号
    pub async fn document_symbols(
        &self,
        file_path: &str,
    ) -> LspResult<Vec<lsp_types::SymbolInformation>> {
        let uri = file_path_to_uri(&self.project_root, file_path)?;
        let params = serde_json::json!({
            "textDocument": { "uri": uri }
        });
        let result = self
            .request("textDocument/documentSymbol", Some(params), std::time::Duration::from_secs(10))
            .await?;
        if result.is_null() {
            return Ok(Vec::new());
        }
        // 可能是 Vec<SymbolInformation> 或 Vec<DocumentSymbol>（层次结构）
        if let Ok(symbols) = serde_json::from_value::<Vec<lsp_types::SymbolInformation>>(result.clone()) {
            Ok(symbols)
        } else if let Ok(doc_symbols) = serde_json::from_value::<Vec<lsp_types::DocumentSymbol>>(result) {
            Ok(flatten_document_symbols(&doc_symbols))
        } else {
            Ok(Vec::new())
        }
    }

    /// 处理从 stdout 读取到的单条消息
    pub(crate) fn dispatch_message(&self, line: &str) {
        let v: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return,
        };

        // 如果有 id 且没有 method → 是 response
        if v.get("id").is_some() && v.get("method").is_none() {
            if let Some(id) = v.get("id").and_then(|i| i.as_u64()) {
                if let Some((_, tx)) = self.inner.pending.remove(&id) {
                    let _ = tx.send(v);
                }
            }
            return;
        }

        // 处理 notification
        if let Some(method) = v.get("method").and_then(|m| m.as_str()) {
            match method {
                "textDocument/publishDiagnostics" => {
                    if let Some(params) = v.get("params") {
                        if let Ok(p) = serde_json::from_value::<PublishDiagnosticsParams>(params.clone()) {
                            let uri = p.uri.to_string();
                            if p.diagnostics.is_empty() {
                                self.inner.diagnostics.remove(&uri);
                            } else {
                                self.inner.diagnostics.insert(uri, p.diagnostics);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

// ====== 辅助函数 ======

/// 文件系统路径 → file:// URI
fn path_to_uri(path: &PathBuf) -> LspResult<String> {
    let canonical = std::fs::canonicalize(path)
        .map_err(|e| super::error::LspError::Protocol(format!("canonicalize 路径失败: {}", e)))?;
    let url = url::Url::from_file_path(&canonical).map_err(|_| {
        super::error::LspError::Protocol(format!("路径转 URI 失败: {}", canonical.display()))
    })?;
    Ok(url.to_string())
}

/// 相对文件路径 → file:// URI
fn file_path_to_uri(project_root: &PathBuf, file_path: &str) -> LspResult<String> {
    let full = project_root.join(file_path);
    path_to_uri(&full)
}

/// 将层次化 DocumentSymbol 展平为扁平的 SymbolInformation 列表
fn flatten_document_symbols(symbols: &[lsp_types::DocumentSymbol]) -> Vec<lsp_types::SymbolInformation> {
    let mut result = Vec::new();
    for sym in symbols {
        let uri_str = path_to_uri_for_symbol(sym);
        result.push(lsp_types::SymbolInformation {
            name: sym.name.clone(),
            kind: sym.kind,
            tags: sym.tags.clone(),
            deprecated: None,
            location: lsp_types::Location {
                uri: uri_str.parse::<lsp_types::Uri>().unwrap_or_else(|_| {
                    "file:///".parse::<lsp_types::Uri>().unwrap()
                }),
                range: sym.range,
            },
            container_name: None,
        });
        if let Some(children) = &sym.children {
            result.extend(flatten_document_symbols(children));
        }
    }
    result
}

fn path_to_uri_for_symbol(_sym: &lsp_types::DocumentSymbol) -> String {
    // DocumentSymbol 不携带完整 URI；返回占位符
    "file:///".to_string()
}
/// 在后台持续读取 stdout，每解析到一帧 LSP 消息就分发到 LspClient
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
                    // 检查是否是诊断发布通知，转发到前端事件
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

// ====== Windows 命令解析 ======

#[cfg(windows)]
fn resolve_lsp_command(command: &str) -> Option<String> {
    use std::path::{Path, PathBuf};

    let p = Path::new(command);
    if p.extension().is_some() {
        return None;
    }
    let file_name = p.file_name()?;

    let pathext =
        std::env::var("PATHEXT").unwrap_or_else(|_| ".EXE;.CMD;.BAT;.COM".to_string());
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
