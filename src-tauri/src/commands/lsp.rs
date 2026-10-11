//! LSP-related Tauri commands
//!
//! Provides language server start, stop, diagnostics query, and auto-detection.

use crate::plugins::lsp::{LspClient, LspManager};
use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

use crate::core::events as ev;
use crate::ierr;

/// Start a language server
///
/// Starts a language server process for the given project and language, completing the LSP handshake.
#[tauri::command]
pub async fn start_lsp_server(
    app: AppHandle,
    project_path: String,
    language_id: String,
) -> Result<Value, String> {
    let lsp_mgr = app.state::<LspManager>();
    let project_root = PathBuf::from(&project_path);

    // Check whether it is already running
    if lsp_mgr.get_client(&project_root, &language_id).is_some() {
        return Ok(serde_json::json!({
            "status": "already_running",
            "languageId": language_id,
        }));
    }

    // Look up the language configuration
    let lang_config = lsp_mgr
        .languages
        .iter()
        .find(|l| l.language_id == language_id)
        .cloned()
        .ok_or_else(|| ierr!("不支持的语言: {}", "Unsupported language: {}", language_id))?;

    // Start the language server
    let sink = crate::plugins::lsp::app_event_sink(&app);
    let client = LspClient::start(
        language_id.clone(),
        project_root.clone(),
        &lang_config.command,
        &lang_config.args,
        sink,
    )
    .await
    .map_err(|e| {
        ierr!(
            "启动语言服务器失败: {}",
            "Failed to start language server: {}",
            e
        )
    })?;

    // Register with the manager
    lsp_mgr.register_client(&project_root, &language_id, client);

    // Notify the frontend
    let _ = app.emit(
        ev::LSP_SERVER_STATUS,
        serde_json::json!({
            "languageId": language_id,
            "projectPath": project_path,
            "status": "started",
        }),
    );

    Ok(serde_json::json!({
        "status": "started",
        "languageId": language_id,
    }))
}

/// Stop a language server
#[tauri::command]
pub async fn stop_lsp_server(
    app: AppHandle,
    project_path: String,
    language_id: String,
) -> Result<Value, String> {
    let lsp_mgr = app.state::<LspManager>();
    let project_root = PathBuf::from(&project_path);

    lsp_mgr
        .remove_client(&project_root, &language_id)
        .await
        .map_err(|e| {
            ierr!(
                "停止语言服务器失败: {}",
                "Failed to stop language server: {}",
                e
            )
        })?;

    let _ = app.emit(
        ev::LSP_SERVER_STATUS,
        serde_json::json!({
            "languageId": language_id,
            "projectPath": project_path,
            "status": "stopped",
        }),
    );

    Ok(serde_json::json!({
        "status": "stopped",
        "languageId": language_id,
    }))
}

/// Get diagnostics
///
/// - `file_path`: optional; if given, only that file's diagnostics are returned, otherwise all files
/// - `severity`: optional; filters "error" / "warning" / "info" / "hint" / "all"
#[tauri::command]
pub async fn get_diagnostics(
    app: AppHandle,
    project_path: String,
    file_path: Option<String>,
    severity: Option<String>,
) -> Result<Value, String> {
    let lsp_mgr = app.state::<LspManager>();
    let project_root = PathBuf::from(&project_path);

    let diags_map = lsp_mgr.get_all_diagnostics(&project_root, file_path.as_deref());

    // Filter by severity
    let severity_filter = severity.unwrap_or_else(|| "all".to_string());

    let mut result_files = Vec::new();
    let mut total_errors = 0u32;
    let mut total_warnings = 0u32;

    for (uri, diags) in &diags_map {
        let filtered: Vec<&lsp_types::Diagnostic> = diags
            .iter()
            .filter(|d| match severity_filter.as_str() {
                "error" => d.severity == Some(lsp_types::DiagnosticSeverity::ERROR),
                "warning" => d.severity == Some(lsp_types::DiagnosticSeverity::WARNING),
                "info" => d.severity == Some(lsp_types::DiagnosticSeverity::INFORMATION),
                "hint" => d.severity == Some(lsp_types::DiagnosticSeverity::HINT),
                _ => true,
            })
            .collect();

        if filtered.is_empty() && severity_filter != "all" {
            continue;
        }

        // Accumulate totals
        for d in &filtered {
            if d.severity == Some(lsp_types::DiagnosticSeverity::ERROR) || d.severity.is_none() {
                total_errors += 1;
            } else if d.severity == Some(lsp_types::DiagnosticSeverity::WARNING) {
                total_warnings += 1;
            }
        }

        // Convert to a serializable format
        let items: Vec<Value> = filtered
            .iter()
            .map(|d| {
                serde_json::json!({
                    "range": {
                        "start": { "line": d.range.start.line, "character": d.range.start.character },
                        "end": { "line": d.range.end.line, "character": d.range.end.character },
                    },
                    "severity": match d.severity {
                        Some(lsp_types::DiagnosticSeverity::ERROR) => "error",
                        Some(lsp_types::DiagnosticSeverity::WARNING) => "warning",
                        Some(lsp_types::DiagnosticSeverity::INFORMATION) => "info",
                        Some(lsp_types::DiagnosticSeverity::HINT) => "hint",
                        _ => "error",
                    },
                    "message": d.message,
                    "source": d.source,
                    "code": d.code.as_ref().map(|c| match c {
                        lsp_types::NumberOrString::Number(n) => n.to_string(),
                        lsp_types::NumberOrString::String(s) => s.clone(),
                    }),
                })
            })
            .collect();

        // Convert the URI to a relative path
        let file_path_str = uri_to_relative(&project_root, uri);

        result_files.push(serde_json::json!({
            "uri": uri,
            "filePath": file_path_str,
            "diagnostics": items,
        }));
    }

    Ok(serde_json::json!({
        "files": result_files,
        "totalErrors": total_errors,
        "totalWarnings": total_warnings,
    }))
}

/// Auto-detect which language servers to start for a project
#[tauri::command]
pub async fn auto_detect_ls(app: AppHandle, project_path: String) -> Result<Value, String> {
    let lsp_mgr = app.state::<LspManager>();
    let project_root = PathBuf::from(&project_path);

    let detected = lsp_mgr.auto_detect(&project_root);

    let configs: Vec<Value> = detected
        .iter()
        .map(|c| {
            serde_json::json!({
                "languageId": c.language_id,
                "name": c.name,
                "command": c.command,
                "args": c.args,
                "markerFiles": c.marker_files,
                "extensions": c.extensions,
            })
        })
        .collect();

    Ok(serde_json::json!({ "languages": configs }))
}

/// Get the list of supported languages
#[tauri::command]
pub async fn list_supported_languages(app: AppHandle) -> Result<Value, String> {
    let lsp_mgr = app.state::<LspManager>();
    let languages = lsp_mgr.list_supported_languages();

    let configs: Vec<Value> = languages
        .iter()
        .map(|c| {
            serde_json::json!({
                "languageId": c.language_id,
                "name": c.name,
                "command": c.command,
                "args": c.args,
                "markerFiles": c.marker_files,
                "extensions": c.extensions,
            })
        })
        .collect();

    Ok(serde_json::json!({ "languages": configs }))
}

/// Stop all language servers for the given project
#[tauri::command]
pub async fn stop_all_lsp_servers(app: AppHandle, project_path: String) -> Result<Value, String> {
    let lsp_mgr = app.state::<LspManager>();
    let project_root = PathBuf::from(&project_path);
    let project_prefix = project_root.to_string_lossy().to_string();

    // Collect the servers to stop
    let to_remove: Vec<String> = lsp_mgr
        .clients
        .iter()
        .filter(|e| {
            let key: &str = e.key();
            key.starts_with(&project_prefix)
        })
        .map(|e| e.value().language_id.clone())
        .collect();

    for lang_id in &to_remove {
        let _ = lsp_mgr.remove_client(&project_root, lang_id).await;
    }

    Ok(serde_json::json!({
        "stopped": to_remove.len(),
    }))
}

/// Convert a file:// URI to a path relative to the project root
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
