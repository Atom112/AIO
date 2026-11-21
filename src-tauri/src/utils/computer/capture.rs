//! 桌面窗口枚举与截图（xcap 的薄封装）。
//!
//! 边界：只产出 `UiWindow`（元信息）与 PNG 文件路径；不解析 UI 语义（那是 `backend`）。
//! 单窗口截图失败（最小化/被遮挡/TCC 拒绝）时回退到该窗口中心所在显示器的可见截图，
//! 保证模型拿到的永远是「屏幕上真实可见的东西」而不是黑图。

use std::path::{Path, PathBuf};

use xcap::image::imageops;
use xcap::{Monitor, Window};

use super::{Rect, UiWindow};
use crate::ierr;

/// 截图的最长边（超过即等比缩小，控制视觉模型的 token 与文件体积）。
const MAX_CAPTURE_EDGE: u32 = 1600;

/// 保留的截图数量（超出按文件名顺序删最旧的）。
const KEEP_SCREENSHOTS: usize = 20;

/// 枚举所有可见窗口（前台窗口置顶，其余保持系统 Z 序）。
///
/// 单个窗口的属性读取失败只跳过该窗口，不影响整体枚举；`Window::all()` 失败才算错误。
pub(crate) fn list_windows() -> Result<Vec<UiWindow>, String> {
    let windows = Window::all()
        .map_err(|e| ierr!("枚举窗口失败: {}", "Failed to enumerate windows: {}", e))?;
    let mut out: Vec<UiWindow> = Vec::with_capacity(windows.len());
    for window in windows {
        let Ok(id) = window.id() else { continue };
        let Ok(pid) = window.pid() else { continue };
        let app_name = window.app_name().unwrap_or_default();
        let title = window.title().unwrap_or_default();
        if title.is_empty() && app_name.is_empty() {
            continue;
        }
        let (Ok(x), Ok(y), Ok(width), Ok(height)) =
            (window.x(), window.y(), window.width(), window.height())
        else {
            continue;
        };
        out.push(UiWindow {
            id,
            pid,
            app_name,
            title,
            rect: Rect {
                x,
                y,
                width: width as i32,
                height: height as i32,
            },
            minimized: window.is_minimized().unwrap_or(false),
            focused: window.is_focused().unwrap_or(false),
        });
    }
    // 稳定分区：前台窗口到最前，其余保持系统给出的 Z 序
    out.sort_by_key(|w| !w.focused);
    Ok(out)
}

/// 屏幕信息行（显示器数量 + 主屏尺寸）；失败降级为「未知」。
pub(crate) fn screen_header() -> String {
    let monitors = match Monitor::all() {
        Ok(list) => list,
        Err(_) => return ierr!("未知", "unknown"),
    };
    let count = monitors.len();
    let primary = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| monitors.first());
    match primary {
        Some(monitor) => match (monitor.width(), monitor.height()) {
            (Ok(w), Ok(h)) => ierr!(
                "{}x{} (主屏), 显示器 {} 台",
                "{}x{} (primary), {} displays",
                w,
                h,
                count
            ),
            _ => ierr!("显示器 {} 台", "{} displays", count),
        },
        None => ierr!("未知", "unknown"),
    }
}

/// 截取窗口（找不到/失败时回退到窗口中心所在显示器），缩放后落盘并清理旧文件。
pub(crate) fn capture_png(window: &UiWindow, dir: &Path) -> Result<PathBuf, String> {
    let image = match capture_window(window) {
        Some(image) => image,
        None => capture_monitor(window)?,
    };
    let image = downscale(image);
    std::fs::create_dir_all(dir).map_err(|e| {
        ierr!(
            "创建截图目录失败: {}",
            "Failed to create screenshot directory: {}",
            e
        )
    })?;
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("{millis}.png"));
    image
        .save(&path)
        .map_err(|e| ierr!("保存截图失败: {}", "Failed to save screenshot: {}", e))?;
    prune_screenshots(dir, KEEP_SCREENSHOTS);
    Ok(path)
}

/// 在最新窗口列表里按 id 找回窗口并截图；窗口不可截时返回 `None`（交给显示器回退）。
fn capture_window(window: &UiWindow) -> Option<xcap::image::RgbaImage> {
    let windows = Window::all().ok()?;
    let target = windows
        .into_iter()
        .find(|w| w.id().ok() == Some(window.id))?;
    target.capture_image().ok()
}

/// 显示器回退：先找窗口中心所在的显示器，再找主屏，最后任意一台。
fn capture_monitor(window: &UiWindow) -> Result<xcap::image::RgbaImage, String> {
    let (cx, cy) = (
        window.rect.x + window.rect.width / 2,
        window.rect.y + window.rect.height / 2,
    );
    if let Ok(monitor) = Monitor::from_point(cx, cy) {
        if let Ok(image) = monitor.capture_image() {
            return Ok(image);
        }
    }
    let monitors = Monitor::all().map_err(|e| {
        ierr!(
            "截屏失败: {}（如为权限问题，请在 系统设置 → 隐私与安全性 → 屏幕录制 中允许 AIO）",
            "Screenshot failed: {} (if this is a permission issue, allow AIO in System Settings → Privacy & Security → Screen Recording)",
            e
        )
    })?;
    let fallback = monitors
        .iter()
        .find(|m| m.is_primary().unwrap_or(false))
        .or_else(|| monitors.first())
        .ok_or_else(|| ierr!("没有可用的显示器", "No display available"))?;
    fallback.capture_image().map_err(|e| {
        ierr!(
            "截屏失败: {}（如为权限问题，请在 系统设置 → 隐私与安全性 → 屏幕录制 中允许 AIO）",
            "Screenshot failed: {} (if this is a permission issue, allow AIO in System Settings → Privacy & Security → Screen Recording)",
            e
        )
    })
}

/// 超过 `MAX_CAPTURE_EDGE` 的截图等比缩小（缩略图质量的快速重采样）。
fn downscale(image: xcap::image::RgbaImage) -> xcap::image::RgbaImage {
    let (width, height) = (image.width(), image.height());
    if width <= MAX_CAPTURE_EDGE && height <= MAX_CAPTURE_EDGE {
        return image;
    }
    let scale = MAX_CAPTURE_EDGE as f32 / width.max(height) as f32;
    let target_w = ((width as f32 * scale).round() as u32).max(1);
    let target_h = ((height as f32 * scale).round() as u32).max(1);
    imageops::thumbnail(&image, target_w, target_h)
}

/// 只保留最新的 `keep` 个 PNG（文件名按时间戳递增），错误忽略。
pub(crate) fn prune_screenshots(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("png"))
        .collect();
    if files.len() <= keep {
        return;
    }
    files.sort();
    let remove_count = files.len() - keep;
    for path in files.into_iter().take(remove_count) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_keeps_newest() {
        let dir = std::env::temp_dir().join(format!("aio-computer-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..25u32 {
            std::fs::write(dir.join(format!("{i:013}.png")), b"x").unwrap();
        }
        // 非 PNG 文件不受影响
        std::fs::write(dir.join("note.txt"), b"x").unwrap();
        prune_screenshots(&dir, 20);
        let mut remaining: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".png"))
            .collect();
        remaining.sort();
        assert_eq!(remaining.len(), 20);
        assert!(remaining.contains(&"0000000000005.png".to_string()));
        assert!(!remaining.contains(&"0000000000004.png".to_string()));
        assert!(dir.join("note.txt").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
