//! LSP agent tools — goto definition, find references, hover, document symbols.
//!
//! LSP-driven code intelligence tools for the AI agent:
//! go-to-definition, find references, hover info, and document symbols.

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolSpec};
use crate::ierr;
use crate::plugins::lsp::{LspClient, LspManager};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use tauri::{Emitter, Manager};

use crate::core::events as ev;

// ====== Tool Specs ======

pub fn lsp_definition_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".to_string(),
        function: ToolFunctionSpec {
            name: "lsp_definition".to_string(),
            description: "跳转到指定位置的定义；返回定义所在的文件路径、行号和列号".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "要查询的文件路径（相对于项目根目录）"
                    },
                    "line": {
                        "type": "number",
                        "description": "光标所在行号（从 0 开始）"
                    },
                    "character": {
                        "type": "number",
                        "description": "光标所在列号（从 0 开始）"
                    }
                },
                "required": ["path", "line", "character"]
            }),
        },
    }
}

pub fn lsp_references_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".to_string(),
        function: ToolFunctionSpec {
            name: "lsp_references".to_string(),
            description: "查找指定符号的所有引用；返回引用位置列表".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "要查询的文件路径（相对于项目根目录）"
                    },
                    "line": {
                        "type": "number",
                        "description": "光标所在行号（从 0 开始）"
                    },
                    "character": {
                        "type": "number",
                        "description": "光标所在列号（从 0 开始）"
                    }
                },
                "required": ["path", "line", "character"]
            }),
        },
    }
}

pub fn lsp_hover_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".to_string(),
        function: ToolFunctionSpec {
            name: "lsp_hover".to_string(),
            description: "查看指定位置的类型/文档信息；返回 hover 提示内容".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "要查询的文件路径（相对于项目根目录）"
                    },
                    "line": {
                        "type": "number",
                        "description": "光标所在行号（从 0 开始）"
                    },
                    "character": {
                        "type": "number",
                        "description": "光标所在列号（从 0 开始）"
                    }
                },
                "required": ["path", "line", "character"]
            }),
        },
    }
}

pub fn lsp_symbols_tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".to_string(),
        function: ToolFunctionSpec {
            name: "lsp_symbols".to_string(),
            description: "列出文件中的所有符号（函数、类、变量等）及其位置".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "要查询的文件路径（相对于项目根目录）"
                    }
                },
                "required": ["path"]
            }),
        },
    }
}

/// Returns every LSP tool spec (the existing `read_lints` plus the four tools above).
pub fn get_all_lsp_tool_specs() -> Vec<ToolSpec> {
    vec![
        super::lsp_tools::tool_spec(),
        lsp_definition_tool_spec(),
        lsp_references_tool_spec(),
        lsp_hover_tool_spec(),
        lsp_symbols_tool_spec(),
    ]
}

// ====== Helpers ======

// tool_ok is provided centrally by utils::tool_result (single source repo-wide)
use crate::utils::tool_result::tool_ok;

/// Maps a file extension to a language server ID.
fn extension_to_language_id(ext: &str) -> Option<&str> {
    match ext {
        "rs" => Some("rust"),
        "ts" | "tsx" => Some("typescript"),
        "js" | "jsx" | "mjs" | "cjs" => Some("typescript"),
        "py" | "pyi" => Some("python"),
        "go" => Some("go"),
        _ => None,
    }
}

/// Relative file path → file:// URI (mirrors the same-named private helper in client.rs).
/// SEC-11: the model-provided path is sandboxed through `file_tools::safe_path`
/// before conversion — `Path::join` lets an absolute argument replace the base
/// entirely and `..` walks out, which would hand the language server (and its
/// hover/definition results) files outside the project.
pub(crate) fn file_path_to_uri(project_root: &Path, file_path: &str) -> Result<String, String> {
    let canonical =
        crate::utils::file_tools::safe_path(&project_root.to_string_lossy(), file_path, true)
            .map_err(|e| {
                ierr!(
                    "文件路径校验失败（{}）: {}",
                    "File path validation failed ({}): {}",
                    file_path,
                    e
                )
            })?;
    let url = url::Url::from_file_path(&canonical).map_err(|_| {
        ierr!(
            "路径转 URI 失败: {}",
            "Failed to convert path to URI: {}",
            canonical.display()
        )
    })?;
    Ok(url.to_string())
}

/// Shared front half: file extension → language_id, plus relative-path normalization.
pub(crate) fn resolve_language_and_path(
    project_root: &str,
    file_path: &str,
) -> Result<(String, String), String> {
    // Extract the extension and map it to a language_id
    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let language_id = extension_to_language_id(ext).ok_or_else(|| {
        ierr!(
            "不支持的文件类型 .{}：没有对应的语言服务器。支持的类型: .rs, .ts, .tsx, .js, .jsx, .py, .go。",
            "Unsupported file type .{}: no matching language server. Supported types: .rs, .ts, .tsx, .js, .jsx, .py, .go.",
            ext
        )
    })?;

    // Normalize the relative path (strip the project_root prefix and leading separators)
    let normalized = file_path
        .strip_prefix(project_root)
        .unwrap_or(file_path)
        .trim_start_matches('/')
        .trim_start_matches('\\')
        .to_string();

    Ok((language_id.to_string(), normalized))
}

/// Per-(project, language) start mutexes: the LSP tools are parallel-safe and run
/// concurrently in one tool block, so simultaneous first-calls must not
/// double-spawn the same language server.
static LSP_START_LOCKS: Lazy<DashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>> =
    Lazy::new(DashMap::new);

/// Install hint per known language-server command (surfaced in start-failure
/// errors so the agent can tell the user exactly how to fix it).
fn language_server_install_hint(command: &str) -> Option<&'static str> {
    match command {
        "pyright-langserver" => Some("npm install -g pyright"),
        "typescript-language-server" => {
            Some("npm install -g typescript typescript-language-server")
        }
        "rust-analyzer" => Some("rustup component add rust-analyzer"),
        "gopls" => Some("go install golang.org/x/tools/gopls@latest"),
        _ => None,
    }
}

/// Resolves (or lazily starts) the `LspClient` for a project + language.
///
/// The agent cannot call the frontend's `auto_detect_ls`/`start_lsp_server`
/// commands — the old "请先使用 auto_detect_ls 启动" error was a dead end that
/// made the whole LSP tool chain unusable whenever the server had not already
/// been started at project open. Now the server is started on demand; only a
/// start FAILURE (typically: not installed) is an error.
async fn ensure_lsp_client(
    app: &tauri::AppHandle,
    project_root: &str,
    language_id: &str,
) -> Result<LspClient, String> {
    let lsp_mgr = app.state::<LspManager>();
    let client_key = format!("{}/{}", project_root, language_id);

    // Fast path: already running
    if let Some(entry) = lsp_mgr.clients.get(&client_key) {
        return Ok(entry.value().clone());
    }

    // Serialize starts for the same key (the entry guard is dropped before awaiting)
    let lock = LSP_START_LOCKS
        .entry(client_key.clone())
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .value()
        .clone();
    let _guard = lock.lock().await;

    // Double-check: a concurrent call may have started it while we waited
    if let Some(entry) = lsp_mgr.clients.get(&client_key) {
        return Ok(entry.value().clone());
    }

    let lang_config = lsp_mgr
        .languages
        .iter()
        .find(|l| l.language_id == language_id)
        .cloned()
        .ok_or_else(|| ierr!("不支持的语言: {}", "Unsupported language: {}", language_id))?;

    let root = Path::new(project_root).to_path_buf();
    let sink = crate::plugins::lsp::app_event_sink(app);
    let client = LspClient::start(
        language_id.to_string(),
        root.clone(),
        &lang_config.command,
        &lang_config.args,
        sink,
    )
    .await
    .map_err(|e| {
        match language_server_install_hint(&lang_config.command) {
            Some(hint) => ierr!(
                "按需启动语言服务器 {} 失败（可能未安装，可先安装: {}）: {}",
                "Failed to start language server {} on demand (likely not installed; install via: {}): {}",
                lang_config.command,
                hint,
                e
            ),
            None => ierr!(
                "按需启动语言服务器 {} 失败: {}",
                "Failed to start language server {} on demand: {}",
                lang_config.command,
                e
            ),
        }
    })?;

    lsp_mgr.register_client(&root, language_id, client.clone());

    // Mirror the frontend auto-start path so the UI reflects the new server
    let _ = app.emit(
        ev::LSP_SERVER_STATUS,
        json!({
            "languageId": language_id,
            "projectPath": project_root,
            "status": "started",
        }),
    );

    Ok(client)
}

/// Passive client lookup by language_id: never starts a server.
/// Shared by the background code-indexing path and read_lints' document sync.
pub(crate) fn passive_lsp_client(
    app: &tauri::AppHandle,
    project_root: &str,
    language_id: &str,
) -> Option<LspClient> {
    let lsp_mgr = app.state::<LspManager>();
    let client_key = format!("{}/{}", project_root, language_id);
    lsp_mgr.clients.get(&client_key).map(|e| e.value().clone())
}

/// Passive lookup: resolves the `LspClient` without ever starting a server.
/// Used by best-effort background paths (memory code indexing falls back to
/// line-window chunking when LSP is absent).
fn resolve_lsp_client_passive(
    app: &tauri::AppHandle,
    project_root: &str,
    file_path: &str,
) -> Result<(LspClient, String), String> {
    let (language_id, normalized) = resolve_language_and_path(project_root, file_path)?;

    // Build the client key ("{project_root}/{language_id}")
    let client_key = format!("{}/{}", project_root, language_id);
    let client = passive_lsp_client(app, project_root, &language_id).ok_or_else(|| {
        ierr!(
            "{} 语言服务器未运行 (key={})。",
            "{} language server is not running (key={}).",
            language_id,
            client_key
        )
    })?;

    Ok((client, normalized))
}

/// Agent-tool lookup: resolves the `LspClient` for a given file path, starting
/// the language server on demand when none is running (see `ensure_lsp_client`).
/// Returns `(client, language_id, relative_path)`.
async fn resolve_lsp_client(
    app: &tauri::AppHandle,
    project_root: &str,
    file_path: &str,
) -> Result<(LspClient, String, String), String> {
    let (language_id, normalized) = resolve_language_and_path(project_root, file_path)?;
    let client = ensure_lsp_client(app, project_root, &language_id).await?;
    Ok((client, language_id, normalized))
}

/// Best-effort document sync (didOpen/didChange with the current disk content)
/// before querying a file — see `LspClient::sync_document`. Read failures
/// (binary / unreadable / missing) skip the sync silently; the query still runs.
async fn sync_document_for_query(
    client: &LspClient,
    project_root: &str,
    relative_path: &str,
    language_id: &str,
    uri: &str,
) {
    let Ok(canonical) = crate::utils::file_tools::safe_path(project_root, relative_path, true)
    else {
        return;
    };
    let Ok(text) = tokio::fs::read_to_string(&canonical).await else {
        return;
    };
    if let Err(e) = client.sync_document(uri, language_id, &text).await {
        tracing::warn!("[lsp] 文档同步失败（{}）: {}", relative_path, e);
    }
}

/// Extracts (line, character) from args.
fn extract_position(args: &Value) -> Result<(u32, u32), String> {
    let line = args
        .get("line")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ierr!("缺少 'line' 参数", "Missing 'line' argument"))?;
    let character = args
        .get("character")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ierr!("缺少 'character' 参数", "Missing 'character' argument"))?;
    Ok((line as u32, character as u32))
}

/// Converts a file:// URI to a readable path string.
fn format_uri(uri: &str) -> String {
    if let Ok(parsed) = url::Url::parse(uri) {
        parsed
            .to_file_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| uri.to_string())
    } else {
        uri.to_string()
    }
}

/// Formats a Position JSON value as "line:col".
fn format_position(pos: &Value) -> String {
    let line = pos.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
    let character = pos.get("character").and_then(|v| v.as_u64()).unwrap_or(0);
    format!("{}:{}", line, character)
}

// ====== Execute Functions ======

pub async fn execute_lsp_definition(
    app: &tauri::AppHandle,
    project_root: &str,
    args: &Value,
) -> Result<ToolResult, String> {
    let file_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ierr!("缺少 'path' 参数", "Missing 'path' argument"))?;
    let (line, character) = extract_position(args)?;

    let (client, language_id, relative_path) =
        resolve_lsp_client(app, project_root, file_path).await?;
    let uri = file_path_to_uri(Path::new(project_root), &relative_path)?;
    sync_document_for_query(&client, project_root, &relative_path, &language_id, &uri).await;

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": line, "character": character }
    });

    let response = client
        .request(
            "textDocument/definition",
            Some(params),
            Duration::from_secs(10),
        )
        .await
        .map_err(|e| {
            ierr!(
                "LSP definition 请求失败: {}",
                "LSP definition request failed: {}",
                e
            )
        })?;

    if response.is_null() {
        return Ok(tool_ok(format!(
            "未找到定义: {file_path}:{line}:{character}"
        )));
    }

    // Prefer parsing as an array of Locations
    if let Some(arr) = response.as_array() {
        if arr.is_empty() {
            return Ok(tool_ok(format!(
                "未找到定义: {file_path}:{line}:{character}"
            )));
        }
        let locations: Vec<String> = arr.iter().map(format_location).collect();
        return Ok(tool_ok(format!(
            "找到 {} 个定义:\n{}",
            locations.len(),
            locations.join("\n")
        )));
    }

    // Then try a single Location object
    if response.get("uri").and_then(|u| u.as_str()).is_some() {
        return Ok(tool_ok(format!("定义位置: {}", format_location(&response))));
    }

    // Fallback: emit the raw result
    Ok(tool_ok(format!(
        "定义查询结果: {}",
        serde_json::to_string_pretty(&response).unwrap_or_else(|_| "(无法序列化)".into())
    )))
}

pub async fn execute_lsp_references(
    app: &tauri::AppHandle,
    project_root: &str,
    args: &Value,
) -> Result<ToolResult, String> {
    let file_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ierr!("缺少 'path' 参数", "Missing 'path' argument"))?;
    let (line, character) = extract_position(args)?;

    let (client, language_id, relative_path) =
        resolve_lsp_client(app, project_root, file_path).await?;
    let uri = file_path_to_uri(Path::new(project_root), &relative_path)?;
    sync_document_for_query(&client, project_root, &relative_path, &language_id, &uri).await;

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": line, "character": character },
        "context": { "includeDeclaration": true }
    });

    let response = client
        .request(
            "textDocument/references",
            Some(params),
            Duration::from_secs(10),
        )
        .await
        .map_err(|e| {
            ierr!(
                "LSP references 请求失败: {}",
                "LSP references request failed: {}",
                e
            )
        })?;

    if response.is_null() {
        return Ok(tool_ok(format!(
            "未找到引用: {file_path}:{line}:{character}"
        )));
    }

    let arr = response.as_array().ok_or_else(|| {
        ierr!(
            "LSP references 返回了意外的响应格式: {}",
            "LSP references returned an unexpected response format: {}",
            response
        )
    })?;

    if arr.is_empty() {
        return Ok(tool_ok(format!(
            "未找到引用: {file_path}:{line}:{character}"
        )));
    }

    let references: Vec<String> = arr.iter().map(format_location).collect();
    Ok(tool_ok(format!(
        "找到 {} 个引用:\n{}",
        references.len(),
        references.join("\n")
    )))
}

pub async fn execute_lsp_hover(
    app: &tauri::AppHandle,
    project_root: &str,
    args: &Value,
) -> Result<ToolResult, String> {
    let file_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ierr!("缺少 'path' 参数", "Missing 'path' argument"))?;
    let (line, character) = extract_position(args)?;

    let (client, language_id, relative_path) =
        resolve_lsp_client(app, project_root, file_path).await?;
    let uri = file_path_to_uri(Path::new(project_root), &relative_path)?;
    sync_document_for_query(&client, project_root, &relative_path, &language_id, &uri).await;

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": line, "character": character }
    });

    let response = client
        .request("textDocument/hover", Some(params), Duration::from_secs(10))
        .await
        .map_err(|e| ierr!("LSP hover 请求失败: {}", "LSP hover request failed: {}", e))?;

    if response.is_null() {
        return Ok(tool_ok(format!(
            "无 hover 信息: {file_path}:{line}:{character}"
        )));
    }

    let contents = response.get("contents");
    let text = match contents {
        None => "无 hover 信息".to_string(),
        Some(c) => extract_hover_text(c),
    };

    Ok(tool_ok(format!(
        "Hover 信息 ({file_path}:{line}:{character}):\n{text}"
    )))
}

pub async fn execute_lsp_symbols(
    app: &tauri::AppHandle,
    project_root: &str,
    args: &Value,
) -> Result<ToolResult, String> {
    let file_path = args
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ierr!("缺少 'path' 参数", "Missing 'path' argument"))?;

    let (client, language_id, relative_path) =
        resolve_lsp_client(app, project_root, file_path).await?;
    let uri = file_path_to_uri(Path::new(project_root), &relative_path)?;
    sync_document_for_query(&client, project_root, &relative_path, &language_id, &uri).await;

    let params = json!({
        "textDocument": { "uri": uri }
    });

    let response = client
        .request(
            "textDocument/documentSymbol",
            Some(params),
            Duration::from_secs(10),
        )
        .await
        .map_err(|e| {
            ierr!(
                "LSP documentSymbol 请求失败: {}",
                "LSP documentSymbol request failed: {}",
                e
            )
        })?;

    if response.is_null() {
        return Ok(tool_ok(format!("文件中无符号: {file_path}")));
    }

    let arr = response.as_array().ok_or_else(|| {
        ierr!(
            "LSP documentSymbol 返回了意外的响应格式: {}",
            "LSP documentSymbol returned an unexpected response format: {}",
            response
        )
    })?;

    if arr.is_empty() {
        return Ok(tool_ok(format!("文件中无符号: {file_path}")));
    }

    let mut lines = Vec::new();
    for symbol in arr {
        flatten_symbol(symbol, 0, &mut lines);
    }

    Ok(tool_ok(format!(
        "文件 {file_path} 中的符号:\n\n{}",
        lines.join("\n")
    )))
}

/// Fetches a file's symbol ranges (for symbol-aware code chunking): (symbol name, start_line, end_line) (LSP lines are 0-based).
/// Returns Err when LSP is not running or the request fails; callers fall back to line-window chunking.
/// Passive by design — never spawns a language server from the background indexing path.
pub async fn fetch_document_symbol_ranges(
    app: &tauri::AppHandle,
    project_root: &str,
    file_path: &str,
) -> Result<Vec<(String, i64, i64)>, String> {
    let (client, relative_path) = resolve_lsp_client_passive(app, project_root, file_path)?;
    let uri = file_path_to_uri(Path::new(project_root), &relative_path)?;
    let params = json!({ "textDocument": { "uri": uri } });
    let response = client
        .request(
            "textDocument/documentSymbol",
            Some(params),
            Duration::from_secs(10),
        )
        .await
        .map_err(|e| {
            ierr!(
                "LSP documentSymbol 请求失败: {}",
                "LSP documentSymbol request failed: {}",
                e
            )
        })?;
    if response.is_null() {
        return Ok(Vec::new());
    }
    let arr = response.as_array().ok_or_else(|| {
        ierr!(
            "LSP documentSymbol 返回了意外的响应格式: {}",
            "LSP documentSymbol returned an unexpected response format: {}",
            response
        )
    })?;
    let mut out: Vec<(String, i64, i64)> = Vec::new();
    fn collect_symbol(symbol: &Value, out: &mut Vec<(String, i64, i64)>) {
        let name = symbol
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("?")
            .to_string();
        // DocumentSymbol uses range{start,end}; SymbolInformation uses location.range
        let range = symbol
            .get("range")
            .or_else(|| symbol.get("location").and_then(|l| l.get("range")));
        let (sl, el) = match range {
            Some(r) => (
                r.get("start")
                    .and_then(|s| s.get("line"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0),
                r.get("end")
                    .and_then(|e| e.get("line"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(0),
            ),
            None => (0, 0),
        };
        if el > sl {
            out.push((name, sl, el));
        }
        if let Some(children) = symbol.get("children").and_then(|c| c.as_array()) {
            for child in children {
                collect_symbol(child, out);
            }
        }
    }
    for symbol in arr {
        collect_symbol(symbol, &mut out);
    }
    Ok(out)
}

// ====== Formatting helpers ======

/// Formats a single Location JSON object as "file:line:col".
fn format_location(loc: &Value) -> String {
    let uri_str = loc.get("uri").and_then(|u| u.as_str()).unwrap_or("?");
    let start = loc.get("range").and_then(|r| r.get("start"));
    let pos = start.map(format_position).unwrap_or_else(|| "?".into());
    format!("  {}:{}", format_uri(uri_str), pos)
}

/// Extracts readable text from hover contents (MarkupContent, MarkedString, or array).
fn extract_hover_text(contents: &Value) -> String {
    // MarkupContent: { kind: "markdown" | "plaintext", value: "..." }
    if let Some(value) = contents.get("value").and_then(|v| v.as_str()) {
        let kind = contents.get("kind").and_then(|k| k.as_str()).unwrap_or("");
        if kind == "markdown" || kind == "plaintext" {
            return value.to_string();
        }
        // MarkedString object: { language: "...", value: "..." }
        let lang = contents
            .get("language")
            .and_then(|l| l.as_str())
            .unwrap_or("");
        if lang.is_empty() {
            return value.to_string();
        }
        return format!("```{lang}\n{value}\n```");
    }

    // MarkedString (plain string)
    if let Some(s) = contents.as_str() {
        return s.to_string();
    }

    // Array of MarkedString / MarkupContent
    if let Some(arr) = contents.as_array() {
        let parts: Vec<String> = arr.iter().map(extract_hover_text).collect();
        return parts.join("\n---\n");
    }

    // Fallback: pretty-print raw JSON
    serde_json::to_string_pretty(contents).unwrap_or_else(|_| "(无法解析)".into())
}

/// Flattens hierarchical DocumentSymbols into indented text lines.
/// Also handles flat SymbolInformation (no children).
fn flatten_symbol(symbol: &Value, depth: usize, out: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    let name = symbol.get("name").and_then(|n| n.as_str()).unwrap_or("?");
    let kind = symbol
        .get("kind")
        .and_then(|k| k.as_u64())
        .map(symbol_kind_name)
        .unwrap_or("?");

    // DocumentSymbol uses selectionRange.start; SymbolInformation uses location.range.start
    let pos = symbol
        .get("selectionRange")
        .or_else(|| symbol.get("range"))
        .and_then(|r| r.get("start"))
        .or_else(|| {
            symbol
                .get("location")
                .and_then(|l| l.get("range"))
                .and_then(|r| r.get("start"))
        })
        .map(format_position)
        .unwrap_or_else(|| "?".into());

    out.push(format!("{indent}{name} {pos} ({kind})"));

    // Recurse into children (only DocumentSymbol has them)
    if let Some(children) = symbol.get("children").and_then(|c| c.as_array()) {
        for child in children {
            flatten_symbol(child, depth + 1, out);
        }
    }
}

/// SymbolKind number → name.
fn symbol_kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "File",
        2 => "Module",
        3 => "Namespace",
        4 => "Package",
        5 => "Class",
        6 => "Method",
        7 => "Property",
        8 => "Field",
        9 => "Constructor",
        10 => "Enum",
        11 => "Interface",
        12 => "Function",
        13 => "Variable",
        14 => "Constant",
        15 => "String",
        16 => "Number",
        17 => "Boolean",
        18 => "Array",
        19 => "Object",
        20 => "Key",
        21 => "Null",
        22 => "EnumMember",
        23 => "Struct",
        24 => "Event",
        25 => "Operator",
        26 => "TypeParameter",
        _ => "Symbol",
    }
}
