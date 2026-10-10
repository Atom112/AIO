//! Commands for exporting / saving to user-chosen paths.
//!
//! Background: the frontend `@tauri-apps/plugin-fs` write permission was previously granted
//! `$HOME/**` (too broad). These named Rust commands consolidate "write to a path chosen via
//! the save dialog", shrinking the privileged surface and leaving room for future validation.
//! Paths come from the frontend `plugin-dialog` `save()`, i.e. an explicit user choice; only basic
//! path validation happens here (absolute path, creatable parent). Copy source paths are additionally restricted to the app attachments directory to prevent arbitrary file reads.

use tauri::Manager;

use crate::ierr;

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

/// Write bytes to a path chosen by the user via the save dialog (HTML/PDF export, image save-as, etc.).
///
/// # Parameters
/// - `path` — target absolute path (produced by `plugin-dialog`'s `save()`)
/// - `bytes` — byte content to write
#[tauri::command]
pub fn save_binary_file(path: String, bytes: Vec<u8>) -> Result<(), String> {
    let p = validate_save_path(&path)?;
    std::fs::write(&p, &bytes).map_err(|e| e.to_string())
}

/// Copy a stored attachment image to a path chosen by the user via the save dialog.
///
/// The source path must be inside the app attachments directory (`app_data/attachments`)
/// to prevent arbitrary file reads. The destination path is produced by `plugin-dialog`'s
/// `save()`, i.e. an explicit user choice.
///
/// # Parameters
/// - `src` — source absolute path (must be inside `app_data/attachments`)
/// - `dest` — target absolute path
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

    let dest_p = validate_save_path(&dest)?;
    std::fs::copy(&src_abs, &dest_p).map_err(|e| e.to_string())?;
    Ok(())
}
