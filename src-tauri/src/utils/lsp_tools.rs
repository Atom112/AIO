//! LSP 诊断工具 — 供 AI Agent 调用的工具定义和执行
//!
//! 提供 `read_lints` 工具，让 Agent 可以查询项目中的 LSP 诊断信息。

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolResultContent, ToolSpec};
use crate::plugins::lsp::LspManager;
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::Manager;

/// 工具 `read_lints` 的 spec（供注册到 Agent 工具列表）
pub fn tool_spec() -> ToolSpec {
    ToolSpec {
        kind: "function".to_string(),
        function: ToolFunctionSpec {
            name: "read_lints".to_string(),
            description: "读取项目中的 LSP 诊断信息（编译错误、类型错误、代码警告等）。可指定文件路径或获取全部诊断。".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "paths": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "要查询的文件路径列表（相对于项目根目录）。不传或为空则返回所有文件的诊断。"
                    },
                    "severity": {
                        "type": "string",
                        "enum": ["error", "warning", "info", "hint", "all"],
                        "description": "过滤诊断级别。默认 \"all\"。"
                    }
                },
                "required": []
            }),
        },
    }
}

fn tool_ok(text: String) -> ToolResult {
    ToolResult {
        content: vec![ToolResultContent {
            kind: "text".into(),
            data: json!({"text": text}),
        }],
        is_error: false,
    }
}

/// 执行 `read_lints` 工具调用
pub async fn execute(
    app: &tauri::AppHandle,
    project_path: &str,
    args: &Value,
) -> Result<ToolResult, String> {
    let lsp_mgr = app.state::<LspManager>();
    let project_root = PathBuf::from(project_path);

    let severity = args
        .get("severity")
        .and_then(|v| v.as_str())
        .unwrap_or("all");

    let file_paths: Vec<String> = args
        .get("paths")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|p| p.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    // 如果没有活跃的语言服务器客户端，返回友好提示
    let project_prefix = project_root.to_string_lossy().to_string();
    let active_clients: Vec<String> = lsp_mgr
        .clients
        .iter()
        .filter(|e| {
            let key: &str = e.key();
            key.starts_with(&project_prefix)
        })
        .map(|e| e.value().language_id.clone())
        .collect();

    if active_clients.is_empty() {
        return Ok(tool_ok(format!(
            "当前项目没有运行中的语言服务器。可用的语言包括：{}。\n请使用 auto_detect_ls 检测项目语言，然后 start_lsp_server 启动对应服务器。",
            lsp_mgr
                .languages
                .iter()
                .map(|l| l.name.as_str())
                .collect::<Vec<_>>()
                .join("、")
        )));
    }

    // 构建输出
    let output = build_diagnostics_output(&lsp_mgr, &project_root, &file_paths, severity);

    Ok(tool_ok(output))
}

fn build_diagnostics_output(
    lsp_mgr: &LspManager,
    project_root: &PathBuf,
    file_paths: &[String],
    severity: &str,
) -> String {
    let mut output = String::new();
    let mut total_errors = 0u32;
    let mut total_warnings = 0u32;

    if !file_paths.is_empty() {
        // 指定文件路径
        for fp in file_paths {
            let diags_map = lsp_mgr.get_all_diagnostics(project_root, Some(fp));
            for (uri, diags) in &diags_map {
                let (text, errs, warns) =
                    format_diagnostics_for_file(project_root, uri, diags, severity);
                output.push_str(&text);
                total_errors += errs;
                total_warnings += warns;
            }
        }
    } else {
        // 所有文件
        let diags_map = lsp_mgr.get_all_diagnostics(project_root, None);
        for (uri, diags) in &diags_map {
            let (text, errs, warns) =
                format_diagnostics_for_file(project_root, uri, diags, severity);
            output.push_str(&text);
            total_errors += errs;
            total_warnings += warns;
        }
    }

    let summary = if total_errors == 0 && total_warnings == 0 {
        "✅ 没有发现诊断问题。".to_string()
    } else {
        format!("📊 共 {} 个错误，{} 个警告\n", total_errors, total_warnings)
    };

    if output.is_empty() {
        format!("{}{}", summary, "没有找到匹配的诊断。")
    } else {
        format!("{}{}", summary, output)
    }
}

fn format_diagnostics_for_file(
    project_root: &PathBuf,
    uri: &str,
    diags: &[lsp_types::Diagnostic],
    severity: &str,
) -> (String, u32, u32) {
    let filtered: Vec<&lsp_types::Diagnostic> = diags
        .iter()
        .filter(|d| match severity {
            "error" => d.severity == Some(lsp_types::DiagnosticSeverity::ERROR),
            "warning" => d.severity == Some(lsp_types::DiagnosticSeverity::WARNING),
            "info" => d.severity == Some(lsp_types::DiagnosticSeverity::INFORMATION),
            "hint" => d.severity == Some(lsp_types::DiagnosticSeverity::HINT),
            _ => true,
        })
        .collect();

    if filtered.is_empty() {
        return (String::new(), 0, 0);
    }

    let rel_path = uri_to_relative(project_root, uri);
    let mut text = format!("\n📄 {}\n", rel_path);
    let mut errs = 0u32;
    let mut warns = 0u32;

    for d in &filtered {
        let sev_icon = match d.severity {
            Some(lsp_types::DiagnosticSeverity::ERROR) | None => "❌",
            Some(lsp_types::DiagnosticSeverity::WARNING) => "⚠️",
            Some(lsp_types::DiagnosticSeverity::INFORMATION) => "ℹ️",
            Some(lsp_types::DiagnosticSeverity::HINT) => "💡",
            _ => "❓",
        };
        let code_str = d
            .code
            .as_ref()
            .map(|c| match c {
                lsp_types::NumberOrString::Number(n) => format!(" ({})", n),
                lsp_types::NumberOrString::String(s) => format!(" ({})", s),
            })
            .unwrap_or_default();

        // 使用 1-based 行号（LSP 是 0-based）
        let line = d.range.start.line + 1;
        let col = d.range.start.character + 1;

        text.push_str(&format!(
            "  {} Line {}, Col {}: {}{}\n",
            sev_icon, line, col, d.message, code_str
        ));

        if d.severity == Some(lsp_types::DiagnosticSeverity::ERROR) || d.severity.is_none() {
            errs += 1;
        } else if d.severity == Some(lsp_types::DiagnosticSeverity::WARNING) {
            warns += 1;
        }
    }

    (text, errs, warns)
}

/// 将 file:// URI 转为相对路径
fn uri_to_relative(project_root: &PathBuf, uri: &str) -> String {
    let path = if let Ok(url) = url::Url::parse(uri) {
        url.to_file_path().ok()
    } else {
        None
    };

    match path {
        Some(abs_path) => {
            if let Ok(relative) = abs_path.strip_prefix(project_root) {
                relative.to_string_lossy().to_string()
            } else {
                abs_path.to_string_lossy().to_string()
            }
        }
        None => uri.to_string(),
    }
}
