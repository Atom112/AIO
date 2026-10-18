//! Feedback and diagnostics module — the alpha-phase user feedback infrastructure
//! (Phase 0 of the feedback plan; see the alpha release plan discussion).
//!
//! Design constraints (from the plan):
//! - **No server, no website**: everything flows through GitHub (Issue Forms /
//!   Discussions) — the snapshot is pasted by the user, the diagnostic package is
//!   uploaded by the user. Nothing leaves the machine without explicit action.
//! - **Privacy-first**: the snapshot contains only what the developer needs to
//!   reproduce an environment (app version, OS, locale, harness tool detection
//!   results). It NEVER contains API keys, session content, file paths outside
//!   well-known config locations, or any user conversation data.
//! - **Structured**: the snapshot is a single-line text the user pastes into the
//!   Issue Form's "环境快照" field (the `feedback.env` i18n hint points here).
//!
//! Two commands:
//! 1. `get_feedback_snapshot` — builds the environment snapshot struct (frontend
//!    renders it and offers copy / pre-filled GitHub links).
//! 2. `export_diagnostic_package` — writes a diagnostic zip (snapshot + config
//!    slot states + backup presence) to the OS downloads dir and returns the path
//!    so the user can attach it to a GitHub issue themselves.

use serde::Serialize;
use std::io::Write;
use tauri::{AppHandle, Manager};
use zip::write::SimpleFileOptions;

use crate::ierr;

// ====== Data structures ======

/// Environment snapshot for user feedback (privacy-safe: no keys, no user data).
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackSnapshot {
    /// App version (e.g. "0.8.3")
    pub app_version: String,
    /// OS identifier: "windows" / "macos" / "linux"
    pub os: String,
    /// OS version (e.g. "Windows 11 26100" / "macOS 15.2" / "Linux 6.2.0")
    pub os_version: String,
    /// CPU architecture: "x86_64" / "aarch64"
    pub arch: String,
    /// Frontend locale: "zh-CN" / "en-US"
    pub locale: String,
    /// Harness tool detection results (tool_id → status), NO paths
    pub harness_tools: Vec<FeedbackToolStatus>,
    /// Local engine scan results (installed engines + versions), NO paths
    pub local_engines: Vec<FeedbackEngineInfo>,
    /// Total system RAM in GB (helps reproduce OOM/perf issues)
    pub ram_gb: f64,
    /// Single-line representation (what the user pastes into the Issue Form)
    pub summary: String,
}

/// One harness tool's detection status (no paths, no versions of specific binaries).
#[derive(Serialize, Clone, Debug)]
pub struct FeedbackToolStatus {
    pub tool_id: String,
    /// "ready" / "installable" / "missing"
    pub status: String,
}

/// One local engine's status (installed only).
#[derive(Serialize, Clone, Debug)]
pub struct FeedbackEngineInfo {
    pub engine_type: String,
    pub version: Option<String>,
}

// ====== OS detection (sysinfo; platform-specific version strings) ======

fn detect_os_version() -> String {
    let os_name = sysinfo::System::name().unwrap_or_else(|| "Unknown".into());
    let os_version = sysinfo::System::long_os_version().unwrap_or_default();
    let kernel = sysinfo::System::kernel_version().unwrap_or_default();
    // Prefer the pretty combined string; fall back piecewise
    if !os_version.is_empty() {
        format!("{os_name} {os_version}").trim().to_string()
    } else if !kernel.is_empty() {
        format!("{os_name} ({kernel})").trim().to_string()
    } else {
        os_name
    }
}

fn detect_ram_gb() -> f64 {
    let sys = sysinfo::System::new_all();
    let bytes = sys.total_memory();
    // Round to 1 decimal (16.0, 32.0, 8.0…)
    let gb = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    (gb * 10.0).round() / 10.0
}

// ====== Snapshot construction ======

/// Build the privacy-safe environment snapshot.
///
/// The caller passes the current frontend locale (the store's `locale` signal) —
/// the backend does not track UI language state itself.
#[tauri::command]
pub fn get_feedback_snapshot(app: AppHandle, locale: String) -> Result<FeedbackSnapshot, String> {
    let app_version = app.package_info().version.to_string();

    // Harness tool detection (reuses the same registry probe the workbench does)
    let harness_tools: Vec<FeedbackToolStatus> =
        crate::plugins::harness::registry::detect_tools(&app)
            .iter()
            .map(|s| FeedbackToolStatus {
                tool_id: s.id.clone(),
                status: s.state.clone(),
            })
            .collect();

    // Local engine scan (installed engines only; no paths)
    let mgr = app.state::<crate::plugins::engine::EngineManager>();
    let plugins = mgr.all_plugins();
    let local_engines: Vec<FeedbackEngineInfo> = plugins
        .into_iter()
        .filter(|p| p.is_platform_supported())
        .filter_map(|p| {
            let info = p.detect_installation(&app);
            if !info.installed {
                return None;
            }
            Some(FeedbackEngineInfo {
                engine_type: p.identifier().to_string(),
                version: info.version,
            })
        })
        .collect();

    let ram_gb = detect_ram_gb();
    let os = std::env::consts::OS.to_string();
    let os_version = detect_os_version();
    let arch = std::env::consts::ARCH.to_string();

    // Compact single-line summary (what the user pastes into the Issue Form)
    let tools_str = harness_tools
        .iter()
        .filter(|t| t.status == "ready")
        .map(|t| t.tool_id.as_str())
        .collect::<Vec<_>>()
        .join("/");
    let engines_str = local_engines
        .iter()
        .map(|e| {
            format!(
                "{}{}",
                e.engine_type,
                e.version
                    .as_deref()
                    .map(|v| format!(":{v}"))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("/");
    let summary = format!(
        "AIO v{app_version} · {os_version} ({arch}) · {locale} · RAM {ram_gb:.1}GB · engines: {engines_str} · tools: {tools_str}",
    );

    Ok(FeedbackSnapshot {
        app_version,
        os,
        os_version,
        arch,
        locale,
        harness_tools,
        local_engines,
        ram_gb,
        summary,
    })
}

// ====== Diagnostic package ======

/// Export a diagnostic zip (snapshot + config slot states) to the user's
/// Downloads directory and return the absolute path. The user attaches the file
/// to a GitHub issue themselves — nothing is uploaded automatically.
///
/// Contents (all privacy-safe):
/// - `snapshot.json` — the full FeedbackSnapshot
/// - `config_slots.txt` — each tool's config file slot state (exists / size /
///   mtime / backup presence) — no contents
/// - `message.txt` — a short README explaining what this file is
#[tauri::command(async)]
pub fn export_diagnostic_package(app: AppHandle, locale: String) -> Result<String, String> {
    export_diagnostic_package_inner(&app, &locale)
}

fn export_diagnostic_package_inner(app: &AppHandle, locale: &str) -> Result<String, String> {
    let snapshot = get_feedback_snapshot(app.clone(), locale.to_string())?;

    // Config slot states (existence/size/mtime — no contents)
    let mut config_report = String::new();
    config_report.push_str("# Harness tool config slot states\n\n");
    for tool_id in [
        "claude", "codex", "opencode", "omp", "gemini", "qwen", "goose", "dsh",
    ] {
        let slots = crate::plugins::harness::toolconf::slots(tool_id)?;
        for slot in &slots {
            config_report.push_str(&format!(
                "[{tool_id}] {} ({}): exists={} size={} modified={} backup={}\n",
                slot.label,
                slot.id,
                slot.exists,
                slot.size_bytes.unwrap_or(0),
                slot.modified.unwrap_or(0),
                slot.has_backup,
            ));
        }
    }

    let message = format!(
        "# AIO 诊断包\n\n版本: v{}\n导出时间: {}\n\n\
         本文件仅包含环境快照与配置文件状态（存在性/大小/修改时间），\
         不含任何 API Key、会话内容或文件内容。\
         请直接拖拽上传到 GitHub Issue。\n",
        snapshot.app_version,
        crate::utils::time::now_iso(),
    );

    // Write to the OS Downloads directory
    let downloads_dir = dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .ok_or_else(|| {
            ierr!(
                "无法确定下载目录",
                "Cannot determine the downloads directory"
            )
        })?;
    let file_name = format!(
        "aio-diagnostics-v{}-{}.zip",
        snapshot.app_version,
        crate::utils::time::now_unix_secs()
    );
    let zip_path = downloads_dir.join(&file_name);

    let file = std::fs::File::create(&zip_path).map_err(|e| format!("创建诊断包文件失败: {e}"))?;
    let mut zip_writer = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // snapshot.json
    zip_writer
        .start_file("snapshot.json", options)
        .map_err(|e| format!("写入 snapshot.json 失败: {e}"))?;
    let snapshot_json = serde_json::to_string_pretty(&snapshot).map_err(|e| e.to_string())?;
    zip_writer
        .write_all(snapshot_json.as_bytes())
        .map_err(|e| format!("写入 snapshot.json 失败: {e}"))?;

    // config_slots.txt
    zip_writer
        .start_file("config_slots.txt", options)
        .map_err(|e| format!("写入 config_slots.txt 失败: {e}"))?;
    zip_writer
        .write_all(config_report.as_bytes())
        .map_err(|e| format!("写入 config_slots.txt 失败: {e}"))?;

    // message.txt
    zip_writer
        .start_file("message.txt", options)
        .map_err(|e| format!("写入 message.txt 失败: {e}"))?;
    zip_writer
        .write_all(message.as_bytes())
        .map_err(|e| format!("写入 message.txt 失败: {e}"))?;

    zip_writer
        .finish()
        .map_err(|e| format!("完成诊断包写入失败: {e}"))?;

    Ok(zip_path.to_string_lossy().into_owned())
}
