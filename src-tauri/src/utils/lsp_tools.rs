//! LSP diagnostics tool — tool definition and execution for the AI Agent
//!
//! Provides the `read_lints` tool so the Agent can query LSP diagnostics for the project.

use crate::core::models::{ToolFunctionSpec, ToolResult, ToolSpec};
use crate::plugins::lsp::LspManager;
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::Manager;

/// Spec of the `read_lints` tool (registered into the Agent tool list)
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

// tool_ok is provided by utils::tool_result (single source of truth repo-wide)
use crate::utils::tool_result::tool_ok;

/// Execute the `read_lints` tool call
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

    // If no active language-server client exists, return a friendly hint
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
        return Ok(tool_ok(
            "当前项目没有运行中的语言服务器（打开项目时会自动检测并启动；未运行通常意味着对应服务器未安装，如 pyright-langserver、rust-analyzer、typescript-language-server、gopls）。\n对具体代码文件调用 lsp_symbols 等 LSP 工具时会按需启动对应服务器。".to_string(),
        ));
    }

    // Best-effort: sync the explicitly requested files into their (already
    // running) language servers — didOpen makes the server analyze the file and
    // publish its diagnostics instead of waiting for the whole workspace scan.
    for fp in &file_paths {
        let Ok((language_id, relative)) =
            crate::utils::lsp_agent_tools::resolve_language_and_path(project_path, fp)
        else {
            continue;
        };
        let Some(client) =
            crate::utils::lsp_agent_tools::passive_lsp_client(app, project_path, &language_id)
        else {
            continue;
        };
        let Ok(uri) = crate::utils::lsp_agent_tools::file_path_to_uri(&project_root, &relative)
        else {
            continue;
        };
        let Ok(canonical) = crate::utils::file_tools::safe_path(project_path, &relative, true)
        else {
            continue;
        };
        if let Ok(text) = tokio::fs::read_to_string(&canonical).await {
            let _ = client.sync_document(&uri, &language_id, &text).await;
        }
    }

    // Diagnostics are published asynchronously after the server (re)analyzes a
    // document — a query racing a freshly started server or a just-opened file
    // would otherwise report "no problems" spuriously. Bounded retry: only when
    // nothing has been published yet AND servers are running.
    let mut output = build_diagnostics_output(&lsp_mgr, &project_root, &file_paths, severity);
    if is_empty_diagnostic_output(&output) {
        for _ in 0..3 {
            tokio::time::sleep(std::time::Duration::from_millis(700)).await;
            output = build_diagnostics_output(&lsp_mgr, &project_root, &file_paths, severity);
            if !is_empty_diagnostic_output(&output) {
                break;
            }
        }
    }

    Ok(tool_ok(output))
}

/// Whether the formatted diagnostics output carries zero problems.
fn is_empty_diagnostic_output(output: &str) -> bool {
    output.starts_with("✅")
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
        // Specified file paths
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
        // All files
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

        // Use 1-based line numbers (LSP is 0-based)
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

/// Convert a file:// URI to a relative path
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
