//! LSP agent tools — goto definition, find references, hover, document symbols.
//!
//! 为 AI Agent 提供 LSP 驱动的代码智能工具：
//! 跳转到定义、查找引用、悬浮类型信息、文档符号列表。

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolResultContent, ToolSpec};
use crate::plugins::lsp::{LspClient, LspManager};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use tauri::Manager;

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

/// 返回所有 LSP 工具 spec（包含已有 `read_lints` 及新增 4 个工具）。
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

fn tool_ok(text: String) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": text}),
        }],
        is_error: false,
    }
}

/// 根据文件扩展名映射到语言服务器 ID。
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

/// 相对文件路径 → file:// URI（仿照 client.rs 中同名私有函数）。
fn file_path_to_uri(project_root: &Path, file_path: &str) -> Result<String, String> {
    let full = project_root.join(file_path);
    let canonical = std::fs::canonicalize(&full)
        .map_err(|e| format!("无法解析路径 {}: {}", full.display(), e))?;
    let url = url::Url::from_file_path(&canonical)
        .map_err(|_| format!("路径转 URI 失败: {}", canonical.display()))?;
    Ok(url.to_string())
}

/// 为给定的文件路径解析 `LspClient`。
///
/// 返回 `(client, relative_path)`；当没有匹配的语言服务器时返回友好错误。
fn resolve_lsp_client(
    app: &tauri::AppHandle,
    project_root: &str,
    file_path: &str,
) -> Result<(LspClient, String), String> {
    let lsp_mgr = app.state::<LspManager>();

    // 提取扩展名并映射到 language_id
    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    let language_id = extension_to_language_id(ext).ok_or_else(|| {
        format!(
            "不支持的文件类型 .{ext}：无法找到对应的语言服务器。支持的类型: .rs, .ts, .tsx, .js, .jsx, .py, .go。请使用 auto_detect_ls 工具启动语言服务器。"
        )
    })?;

    // 构造客户端 key（"{project_root}/{language_id}"）
    let client_key = format!("{}/{}", project_root, language_id);
    let client = lsp_mgr
        .clients
        .get(&client_key)
        .map(|entry| entry.value().clone())
        .ok_or_else(|| {
            format!(
                "未找到 {language_id} 语言服务器客户端 (key={client_key})。请先使用 auto_detect_ls 工具启动语言服务器。"
            )
        })?;

    // 规范化相对路径（去 project_root 前缀 + 前导分隔符）
    let normalized = file_path
        .strip_prefix(project_root)
        .unwrap_or(file_path)
        .trim_start_matches('/')
        .trim_start_matches('\\')
        .to_string();

    Ok((client, normalized))
}

/// 从 args 中提取 (line, character)。
fn extract_position(args: &Value) -> Result<(u32, u32), String> {
    let line = args
        .get("line")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "缺少 'line' 参数".to_string())?;
    let character = args
        .get("character")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "缺少 'character' 参数".to_string())?;
    Ok((line as u32, character as u32))
}

/// 将 file:// URI 转为可读的相对路径字符串。
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

/// 格式化 Position JSON 值为 "行:列"。
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
        .ok_or_else(|| "缺少 'path' 参数".to_string())?;
    let (line, character) = extract_position(args)?;

    let (client, relative_path) = resolve_lsp_client(app, project_root, file_path)?;
    let uri = file_path_to_uri(Path::new(project_root), &relative_path)?;

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
        .map_err(|e| format!("LSP definition 请求失败: {e}"))?;

    if response.is_null() {
        return Ok(tool_ok(format!(
            "未找到定义: {file_path}:{line}:{character}"
        )));
    }

    // 优先解析为 Location 数组
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

    // 再尝试解析为单条 Location
    if response.get("uri").and_then(|u| u.as_str()).is_some() {
        return Ok(tool_ok(format!("定义位置: {}", format_location(&response))));
    }

    // 兜底：直接输出原始结果
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
        .ok_or_else(|| "缺少 'path' 参数".to_string())?;
    let (line, character) = extract_position(args)?;

    let (client, relative_path) = resolve_lsp_client(app, project_root, file_path)?;
    let uri = file_path_to_uri(Path::new(project_root), &relative_path)?;

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
        .map_err(|e| format!("LSP references 请求失败: {e}"))?;

    if response.is_null() {
        return Ok(tool_ok(format!(
            "未找到引用: {file_path}:{line}:{character}"
        )));
    }

    let arr = response
        .as_array()
        .ok_or_else(|| format!("LSP references 返回了意外的响应格式: {response}"))?;

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
        .ok_or_else(|| "缺少 'path' 参数".to_string())?;
    let (line, character) = extract_position(args)?;

    let (client, relative_path) = resolve_lsp_client(app, project_root, file_path)?;
    let uri = file_path_to_uri(Path::new(project_root), &relative_path)?;

    let params = json!({
        "textDocument": { "uri": uri },
        "position": { "line": line, "character": character }
    });

    let response = client
        .request("textDocument/hover", Some(params), Duration::from_secs(10))
        .await
        .map_err(|e| format!("LSP hover 请求失败: {e}"))?;

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
        .ok_or_else(|| "缺少 'path' 参数".to_string())?;

    let (client, relative_path) = resolve_lsp_client(app, project_root, file_path)?;
    let uri = file_path_to_uri(Path::new(project_root), &relative_path)?;

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
        .map_err(|e| format!("LSP documentSymbol 请求失败: {e}"))?;

    if response.is_null() {
        return Ok(tool_ok(format!("文件中无符号: {file_path}")));
    }

    let arr = response
        .as_array()
        .ok_or_else(|| format!("LSP documentSymbol 返回了意外的响应格式: {response}"))?;

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

// ====== Formatting helpers ======

/// 格式化单个 Location JSON 对象为 "文件:行:列"。
fn format_location(loc: &Value) -> String {
    let uri_str = loc.get("uri").and_then(|u| u.as_str()).unwrap_or("?");
    let start = loc.get("range").and_then(|r| r.get("start"));
    let pos = start.map(format_position).unwrap_or_else(|| "?".into());
    format!("  {}:{}", format_uri(uri_str), pos)
}

/// 从 hover contents 中提取可读文本（支持 MarkupContent、MarkedString、数组）。
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

/// 展开层级 DocumentSymbol 为缩进文本行。
/// 同时兼容扁平的 SymbolInformation（无 children）。
fn flatten_symbol(symbol: &Value, depth: usize, out: &mut Vec<String>) {
    let indent = "  ".repeat(depth);
    let name = symbol.get("name").and_then(|n| n.as_str()).unwrap_or("?");
    let kind = symbol
        .get("kind")
        .and_then(|k| k.as_u64())
        .map(symbol_kind_name)
        .unwrap_or("?");

    // DocumentSymbol 用 selectionRange.start；SymbolInformation 用 location.range.start
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

    // 递归子节点（DocumentSymbol 才有 children）
    if let Some(children) = symbol.get("children").and_then(|c| c.as_array()) {
        for child in children {
            flatten_symbol(child, depth + 1, out);
        }
    }
}

/// SymbolKind 数值 → 名称。
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
