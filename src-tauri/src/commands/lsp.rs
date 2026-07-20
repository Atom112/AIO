//! LSP 相关 Tauri 命令
//!
//! 提供语言服务器的启动、停止、诊断查询和自动检测功能。

use crate::plugins::lsp::{LspClient, LspManager};
use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager};

/// 启动语言服务器
///
/// 为指定项目的指定语言启动语言服务器进程，完成 LSP 握手。
#[tauri::command]
pub async fn start_lsp_server(
    app: AppHandle,
    project_path: String,
    language_id: String,
) -> Result<Value, String> {
    let lsp_mgr = app.state::<LspManager>();
    let project_root = PathBuf::from(&project_path);

    // 检查是否已启动
    if lsp_mgr.get_client(&project_root, &language_id).is_some() {
        return Ok(serde_json::json!({
            "status": "already_running",
            "languageId": language_id,
        }));
    }

    // 查找语言配置
    let lang_config = lsp_mgr
        .languages
        .iter()
        .find(|l| l.language_id == language_id)
        .cloned()
        .ok_or_else(|| format!("不支持的语言: {}", language_id))?;

    // 启动语言服务器
    let client = LspClient::start(
        language_id.clone(),
        project_root.clone(),
        &lang_config.command,
        &lang_config.args,
        app.clone(),
    )
    .await
    .map_err(|e| format!("启动语言服务器失败: {}", e))?;

    // 注册到管理器
    lsp_mgr.register_client(&project_root, &language_id, client);

    // 通知前端
    let _ = app.emit(
        "lsp-server-status",
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

/// 停止语言服务器
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
        .map_err(|e| format!("停止语言服务器失败: {}", e))?;

    let _ = app.emit(
        "lsp-server-status",
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

/// 获取诊断信息
///
/// - `file_path`: 可选，指定则只返回该文件的诊断，否则返回所有文件
/// - `severity`: 可选，过滤 "error" / "warning" / "info" / "hint" / "all"
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

    // 过滤 severity
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

        // 统计
        for d in &filtered {
            if d.severity == Some(lsp_types::DiagnosticSeverity::ERROR) || d.severity.is_none() {
                total_errors += 1;
            } else if d.severity == Some(lsp_types::DiagnosticSeverity::WARNING) {
                total_warnings += 1;
            }
        }

        // 转换为可序列化的格式
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

        // 将 URI 转为相对路径
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

/// 自动检测项目应启动的语言服务器
#[tauri::command]
pub async fn auto_detect_ls(
    app: AppHandle,
    project_path: String,
) -> Result<Value, String> {
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

/// 获取支持的语言列表
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

/// 关闭指定项目的所有语言服务器
#[tauri::command]
pub async fn stop_all_lsp_servers(
    app: AppHandle,
    project_path: String,
) -> Result<Value, String> {
    let lsp_mgr = app.state::<LspManager>();
    let project_root = PathBuf::from(&project_path);
    let project_prefix = project_root.to_string_lossy().to_string();

    // 收集需要关闭的服务器
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

/// 将 file:// URI 转为相对于项目根目录的路径
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
