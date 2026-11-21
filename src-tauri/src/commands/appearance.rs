//! Appearance commands: user-customized app background image storage.
//!
//! The frontend only ever stores a path string; the picked image is copied into
//! `<app_data_dir>/backgrounds/<sha256>.<ext>` so the displayed background survives
//! the user moving or deleting the original file, and so the asset protocol only
//! has to expose one well-known directory.
//!
//! Trust boundary: the incoming `path` is an arbitrary frontend string. Reading it
//! is capped to a fixed extension whitelist and 20 MB, the bytes never travel back
//! over IPC (only the destination path does), and deletion is pinned to the
//! canonicalized `backgrounds` directory — a forged path can at worst re-copy a
//! large image file that the process can already read, never delete anything else.

use crate::ierr;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tauri::Manager;

/// Upper bound for a background image (20 MB) — keeps the copy, SHA-256 digest and
/// WebView rasterization of a full-window layer reasonable.
const MAX_BACKGROUND_BYTES: u64 = 20 * 1024 * 1024;

/// Accepted image extensions (lowercase). GIF/SVG are rejected: SVG is scriptable
/// markup and GIF animation in a full-window layer costs a repaint every frame.
const BACKGROUND_EXTENSIONS: [&str; 4] = ["png", "jpg", "jpeg", "webp"];

/// Destination of a background copy inside the `backgrounds` directory.
/// Content-addressed by SHA-256, so re-picking the same image reuses the existing file.
fn background_destination(base: &Path, sha256: &str, ext: &str) -> PathBuf {
    base.join(format!("{sha256}.{ext}"))
}

/// Validate and read a background source file.
/// # Parameters
/// - `path` — source path picked by the user
/// # Returns
/// `(bytes, lowercase extension)` of an existing, non-empty, whitelisted file no
/// larger than [`MAX_BACKGROUND_BYTES`].
fn validate_background_source(path: &Path) -> Result<(Vec<u8>, String), String> {
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !BACKGROUND_EXTENSIONS.contains(&extension.as_str()) {
        return Err(ierr!(
            "不支持的背景图格式: {}（仅支持 png/jpg/jpeg/webp）",
            "Unsupported background image format: {} (png/jpg/jpeg/webp only)",
            if extension.is_empty() {
                "unknown".to_string()
            } else {
                extension
            }
        ));
    }
    let metadata = std::fs::metadata(path)
        .map_err(|e| ierr!("无法读取背景图: {}", "Cannot read background image: {}", e))?;
    if !metadata.is_file() {
        return Err(ierr!(
            "背景图路径不是文件",
            "Background image path is not a file"
        ));
    }
    if metadata.len() == 0 {
        return Err(ierr!("背景图内容为空", "Background image is empty"));
    }
    if metadata.len() > MAX_BACKGROUND_BYTES {
        return Err(ierr!(
            "背景图过大（最大 20MB）",
            "Background image is too large (max 20 MB)"
        ));
    }
    let bytes = std::fs::read(path)
        .map_err(|e| ierr!("无法读取背景图: {}", "Cannot read background image: {}", e))?;
    Ok((bytes, extension))
}

/// Copy a user-picked image into the app data `backgrounds` directory.
/// # Parameters
/// - `app` — app handle (resolves the app data directory)
/// - `path` — absolute path of the source image (frontend dialog result)
/// # Returns
/// Absolute path of the stored copy (`<app_data_dir>/backgrounds/<sha256>.<ext>`).
/// The source file is only read, never moved or modified.
#[tauri::command]
pub async fn save_app_background(app: tauri::AppHandle, path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let (bytes, extension) = validate_background_source(Path::new(&path))?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let directory = app
            .path()
            .app_data_dir()
            .map_err(|e| e.to_string())?
            .join("backgrounds");
        std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
        let destination = background_destination(&directory, &sha256, &extension);
        if !destination.exists() {
            crate::core::fs_util::atomic_write(&destination, &bytes).map_err(|e| e.to_string())?;
        }
        Ok(destination.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Delete a previously stored background image.
/// # Parameters
/// - `app` — app handle (resolves the app data directory)
/// - `path` — stored path returned by [`save_app_background`]
/// # Behavior
/// An empty path, a missing `backgrounds` directory or an already-deleted file are
/// all `Ok(())` (best-effort cleanup). Any path outside the canonicalized
/// `backgrounds` directory is refused.
#[tauri::command]
pub async fn delete_app_background(app: tauri::AppHandle, path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        if path.trim().is_empty() {
            return Ok(());
        }
        let Ok(directory) = app.path().app_data_dir() else {
            return Ok(());
        };
        let Ok(directory) = directory.join("backgrounds").canonicalize() else {
            // No backgrounds directory yet: nothing to delete.
            return Ok(());
        };
        let Ok(candidate) = Path::new(&path).canonicalize() else {
            // Already gone (or unreadable): cleanup is best-effort.
            return Ok(());
        };
        if !candidate.starts_with(&directory) || !candidate.is_file() {
            return Err(ierr!(
                "拒绝删除背景目录之外的文件",
                "Refusing to delete a file outside the backgrounds directory"
            ));
        }
        std::fs::remove_file(&candidate).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create an isolated temp directory for one test.
    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aio-bg-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn accepts_png_file_and_returns_bytes_and_extension() {
        let dir = temp_dir();
        let source = dir.join("wallpaper.PNG");
        std::fs::write(&source, b"fake-png-bytes").expect("write png");
        let (bytes, extension) = validate_background_source(&source).expect("valid png");
        assert_eq!(bytes, b"fake-png-bytes");
        assert_eq!(extension, "png");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_unsupported_extension() {
        let dir = temp_dir();
        let source = dir.join("notes.txt");
        std::fs::write(&source, b"hello").expect("write txt");
        assert!(validate_background_source(&source).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_empty_and_missing_files() {
        let dir = temp_dir();
        let empty = dir.join("empty.png");
        std::fs::write(&empty, b"").expect("write empty");
        assert!(validate_background_source(&empty).is_err());
        assert!(validate_background_source(&dir.join("missing.png")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn destination_is_content_addressed_inside_backgrounds() {
        let base = Path::new("/data/backgrounds");
        assert_eq!(
            background_destination(base, "abc123", "webp"),
            base.join("abc123.webp")
        );
    }
}
