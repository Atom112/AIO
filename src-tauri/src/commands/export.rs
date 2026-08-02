//! 导出 / 保存到用户选定路径的命令（CONF-02）。
//!
//! 背景：前端 `@tauri-apps/plugin-fs` 的写权限此前被授予 `$HOME/**`（过宽）。
//! 这里把“写到用户经保存对话框选择的路径”收口为具名 Rust 命令，缩小凭据面并为后续校验留口。
//! 路径由前端 `plugin-dialog` 的 `save()` 产生，属用户明确选择；此处只做基础路径校验
//! （绝对路径、可创建父目录）。源复制路径额外限制在应用附件目录内，防任意文件读取。

use tauri::Manager;

/// 校验保存路径：非空、绝对路径，并确保父目录存在。
fn validate_save_path(path: &str) -> Result<std::path::PathBuf, String> {
    if path.trim().is_empty() {
        return Err("保存路径不能为空".into());
    }
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        return Err("保存路径必须是绝对路径".into());
    }
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| format!("无法创建目录: {e}"))?;
        }
    }
    Ok(p.to_path_buf())
}

/// 将字节写入用户经保存对话框选择的路径（HTML/PDF 导出、图片另存等）。
///
/// # 参数
/// - `path` — 目标绝对路径（由 `plugin-dialog` 的 `save()` 产生）
/// - `bytes` — 待写入的字节内容
#[tauri::command]
pub fn save_binary_file(path: String, bytes: Vec<u8>) -> Result<(), String> {
    let p = validate_save_path(&path)?;
    std::fs::write(&p, &bytes).map_err(|e| e.to_string())
}

/// 复制应用内已存储的附件图片到用户经保存对话框选择的路径。
///
/// 源路径必须位于应用附件目录（`app_data/attachments`）内，防止任意文件读取。
/// 目标路径由 `plugin-dialog` 的 `save()` 产生，属用户明确选择。
///
/// # 参数
/// - `src` — 源文件绝对路径（须在 `app_data/attachments` 内）
/// - `dest` — 目标绝对路径
#[tauri::command]
pub fn copy_stored_image(app: tauri::AppHandle, src: String, dest: String) -> Result<(), String> {
    let app_data = app.path().app_data_dir().map_err(|e| e.to_string())?;
    let allowed_root = app_data.join("attachments");
    let allowed_root = std::fs::canonicalize(&allowed_root).map_err(|e| e.to_string())?;

    let src_abs = std::fs::canonicalize(std::path::Path::new(&src)).map_err(|e| e.to_string())?;
    if !src_abs.starts_with(&allowed_root) {
        return Err("源文件不在允许的附件目录内".into());
    }

    let dest_p = validate_save_path(&dest)?;
    std::fs::copy(&src_abs, &dest_p).map_err(|e| e.to_string())?;
    Ok(())
}
