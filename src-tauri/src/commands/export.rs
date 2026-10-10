//! Commands for exporting / saving to user-chosen paths.
//!
//! Trust boundary (INFRA-5 / HOR-3, "frontend parameters are untrusted"): a path
//! string crossing the IPC boundary is NOT proof of user intent. The privileged
//! write commands here only accept paths that the user picked through
//! [`pick_save_path`] in this session — the native dialog runs backend-side
//! (tauri-plugin-dialog Rust API), the chosen path is recorded in a session set,
//! and `save_binary_file` / `memory_export_file` refuse everything else.
//!
//! Copy source paths are additionally restricted to the app attachments
//! directory to prevent arbitrary file reads.

use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

use crate::ierr;

// ====== Session-approved dialog paths ======

/// Paths approved by the user via the backend save dialog in this session.
fn approved_save_paths() -> &'static Mutex<HashSet<String>> {
    static S: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Paths approved by the user via the backend open dialog in this session.
fn approved_read_paths() -> &'static Mutex<HashSet<String>> {
    static S: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashSet::new()))
}

/// Record a save path chosen through the backend dialog (exact string form).
pub(crate) fn approve_save_path(path: &str) {
    approved_save_paths()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_string());
}

/// Whether the exact path string was chosen through the backend save dialog.
pub(crate) fn is_approved_save_path(path: &str) -> bool {
    approved_save_paths()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(path)
}

/// Record a read path chosen through the backend open dialog (exact string form).
pub(crate) fn approve_read_path(path: &str) {
    approved_read_paths()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(path.to_string());
}

/// Whether the exact path string was chosen through the backend open dialog.
pub(crate) fn is_approved_read_path(path: &str) -> bool {
    approved_read_paths()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(path)
}

/// Open the native **save** dialog (backend-side) and return the chosen path.
/// The returned path is recorded as session-approved for the privileged write
/// commands; `None` means the user cancelled.
///
/// # Parameters
/// - `default_name` — suggested file name
/// - `filter_name` / `extensions` — optional single file-type filter (e.g. "PDF" / ["pdf"])
#[tauri::command]
pub async fn pick_save_path(
    app: tauri::AppHandle,
    default_name: String,
    filter_name: Option<String>,
    extensions: Option<Vec<String>>,
) -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut builder = app.dialog().file().set_file_name(default_name);
        if let Some(exts) = extensions.as_deref() {
            if !exts.is_empty() {
                let refs: Vec<&str> = exts.iter().map(|s| s.as_str()).collect();
                builder = builder.add_filter(
                    filter_name.unwrap_or_else(|| "Files".to_string()).as_str(),
                    &refs,
                );
            }
        }
        builder.blocking_save_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(picked.and_then(|fp| {
        fp.into_path().ok().map(|p| {
            let s = p.to_string_lossy().into_owned();
            approve_save_path(&s);
            s
        })
    }))
}

/// Open the native **open** dialog (backend-side) and return the chosen file path.
/// The returned path is recorded as session-approved for read commands
/// (e.g. `memory_import_file`); `None` means the user cancelled.
///
/// # Parameters
/// - `filter_name` / `extensions` — optional single file-type filter (e.g. "JSON" / ["json"])
#[tauri::command]
pub async fn pick_open_path(
    app: tauri::AppHandle,
    filter_name: Option<String>,
    extensions: Option<Vec<String>>,
) -> Result<Option<String>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let mut builder = app.dialog().file();
        if let Some(exts) = extensions.as_deref() {
            if !exts.is_empty() {
                let refs: Vec<&str> = exts.iter().map(|s| s.as_str()).collect();
                builder = builder.add_filter(
                    filter_name.unwrap_or_else(|| "Files".to_string()).as_str(),
                    &refs,
                );
            }
        }
        builder.blocking_pick_file()
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(picked.and_then(|fp| {
        fp.into_path().ok().map(|p| {
            let s = p.to_string_lossy().into_owned();
            approve_read_path(&s);
            s
        })
    }))
}

/// Validate a save path: non-empty, absolute, and with a creatable parent directory.
fn validate_save_path(path: &str) -> Result<std::path::PathBuf, String> {
    if path.trim().is_empty() {
        return Err(ierr!("保存路径不能为空", "Save path must not be empty"));
    }
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        return Err(ierr!(
            "保存路径必须是绝对路径",
            "Save path must be absolute"
        ));
    }
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| ierr!("无法创建目录: {}", "Failed to create directory: {}", e))?;
        }
    }
    Ok(p.to_path_buf())
}

/// Write bytes to a path chosen by the user via the backend save dialog
/// (HTML/PDF export, image save-as, etc.).
///
/// # Parameters
/// - `path` — target absolute path (**must** be a path returned by `pick_save_path`
///   in this session; a bare IPC string is refused — INFRA-5)
/// - `bytes` — byte content to write
#[tauri::command]
pub fn save_binary_file(path: String, bytes: Vec<u8>) -> Result<(), String> {
    if !is_approved_save_path(&path) {
        return Err(ierr!(
            "保存路径未经过系统保存对话框确认，已拒绝写入",
            "Save path was not confirmed through the system save dialog; write refused"
        ));
    }
    let p = validate_save_path(&path)?;
    crate::core::fs_util::atomic_write(&p, &bytes).map_err(|e| e.to_string())
}

/// Copy a stored attachment image to a path chosen by the user via the backend save dialog.
///
/// The source path must be inside the app attachments directory (`app_data/attachments`)
/// to prevent arbitrary file reads. The destination path must have been returned by
/// `pick_save_path` in this session.
///
/// # Parameters
/// - `src` — source absolute path (must be inside `app_data/attachments`)
/// - `dest` — target absolute path (session-approved via `pick_save_path`)
#[tauri::command]
pub fn copy_stored_image(app: tauri::AppHandle, src: String, dest: String) -> Result<(), String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let allowed_root = app_data.join("attachments");
    let allowed_root = std::fs::canonicalize(&allowed_root).map_err(|e| e.to_string())?;

    let src_abs = std::fs::canonicalize(std::path::Path::new(&src)).map_err(|e| e.to_string())?;
    if !src_abs.starts_with(&allowed_root) {
        return Err(ierr!(
            "源文件不在允许的附件目录内",
            "Source file is not inside the allowed attachments directory"
        ));
    }

    if !is_approved_save_path(&dest) {
        return Err(ierr!(
            "保存路径未经过系统保存对话框确认，已拒绝写入",
            "Save path was not confirmed through the system save dialog; write refused"
        ));
    }
    let dest_p = validate_save_path(&dest)?;
    std::fs::copy(&src_abs, &dest_p).map_err(|e| e.to_string())?;
    Ok(())
}
