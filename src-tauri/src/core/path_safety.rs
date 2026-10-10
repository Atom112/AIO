//! IPC 前端路径参数的安全校验（“前端参数不可信”守则的实现侧）。
//!
//! 守则：凡 IPC 命令接受“相对某个受控根目录的路径 / ID / 文件名”参数的，
//! 必须经 [`safe_join`] 校验后再拼接。Rust 的 `Path::join` 对绝对路径参数会
//! **整体替换**基路径，`..` 组件也会在 syscall 时逃逸——webview 侧任意 JS
//! （XSS / 投毒依赖）即可借 `invoke` 做任意目录递归删除 / 任意路径写入。
//!
//! 规则（对 Windows 成立的保留字符规则按全平台执行——这些字符出现在受控
//! 目录的相对路径里本就不合法）：
//! - 拒绝空串、绝对路径（含 Windows 盘符 `C:`、根 `\` / `/`、UNC 前缀）
//! - 拒绝任何 `..` 组件（`./` 允许）
//! - 拒绝保留字符 `<>:"|?*`、控制字符、以点/空格结尾的段、保留设备名（CON/NUL/COM1..）
//!
//! 通过校验后所有组件均为 `Normal`，结果保证永远位于 `base` 之内。

use crate::ierr;
use std::path::{Component, Path, PathBuf};

/// Windows 保留设备名（带不带扩展名都保留）
const RESERVED_DEVICE_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// 校验单个路径组件。
fn validate_component(component: &str) -> Result<(), String> {
    if component.is_empty() {
        return Err(ierr!("非法的空路径段", "Illegal empty path segment"));
    }
    if component.len() > 255 {
        return Err(ierr!(
            "路径段过长: {}",
            "Path segment is too long: {}",
            component
        ));
    }
    for ch in component.chars() {
        let invalid = (ch as u32) < 0x20 || matches!(ch, '<' | '>' | ':' | '"' | '|' | '?' | '*');
        if invalid {
            return Err(ierr!(
                "路径段含非法字符 \"{}\": {}",
                "Path segment contains an illegal character \"{}\": {}",
                ch,
                component
            ));
        }
    }
    if component.ends_with('.') || component.ends_with(' ') {
        return Err(ierr!(
            "路径段不能以点或空格结尾: {}",
            "Path segment must not end with a dot or a space: {}",
            component
        ));
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .to_ascii_uppercase();
    if RESERVED_DEVICE_NAMES.contains(&stem.as_str()) {
        return Err(ierr!(
            "路径段为系统保留设备名: {}",
            "Path segment is a reserved device name: {}",
            component
        ));
    }
    Ok(())
}

/// 校验前端传入的相对路径并把结果拼接到 `base` 之下。
///
/// 返回的 [`PathBuf`] 的每个组件都是 `Normal`，因此永远位于 `base` 之内；
/// 校验失败返回用户可读错误（命令层直接 `?` 透传）。
pub fn safe_join(base: &Path, rel: &str) -> Result<PathBuf, String> {
    let trimmed = rel.trim();
    if trimmed.is_empty() {
        return Err(ierr!(
            "路径参数不能为空",
            "Path parameter must not be empty"
        ));
    }
    let rel_path = Path::new(trimmed);
    if rel_path.is_absolute() || rel_path.has_root() {
        return Err(ierr!(
            "路径参数必须为相对路径: {}",
            "Path parameter must be relative: {}",
            trimmed
        ));
    }
    for component in rel_path.components() {
        match component {
            Component::Normal(part) => validate_component(&part.to_string_lossy())?,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ierr!(
                    "路径参数不允许上级目录回溯或盘符前缀: {}",
                    "Path parameter must not contain parent traversal or a drive prefix: {}",
                    trimmed
                ));
            }
        }
    }
    Ok(base.join(trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "/data/models";

    #[test]
    fn accepts_normal_relative_ids() {
        let base = Path::new(BASE);
        assert!(safe_join(base, "Qwen3-14B-Q4_K_M").is_ok());
        assert!(safe_join(base, "user/Qwen2.5-7B/model-Q4.gguf").is_ok());
        assert!(safe_join(base, "./nested/deep/file.bin").is_ok());
        assert_eq!(safe_join(base, "abc").unwrap(), base.join("abc"));
    }

    #[test]
    fn rejects_parent_traversal() {
        let base = Path::new(BASE);
        for evil in ["..", "../etc", "a/../../b", "a/../..", "./..", "a/./../b"] {
            assert!(safe_join(base, evil).is_err(), "应拒绝: {evil}");
        }
    }

    #[test]
    fn rejects_rooted_paths() {
        let base = Path::new(BASE);
        // Unix 绝对路径在两个平台都会被 has_root 拒绝
        for evil in ["/etc/passwd", "/data/models/../../etc"] {
            assert!(safe_join(base, evil).is_err(), "应拒绝: {evil}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn rejects_windows_absolute_and_drive_prefixed() {
        let base = Path::new(r"C:\data\models");
        for evil in [
            r"C:\Users\x\Documents",
            "C:Users\\x\\Documents",
            r"\\server\share\evil",
            "C:\\",
            r"\Windows\System32",
        ] {
            assert!(safe_join(base, evil).is_err(), "应拒绝: {evil}");
        }
    }

    #[test]
    fn rejects_reserved_chars_and_device_names() {
        let base = Path::new(BASE);
        for evil in [
            "a<b",
            "a>b",
            "a:b",
            "a\"b",
            "a|b",
            "a?b",
            "a*b",
            "CON",
            "con.txt",
            "NUL.bin",
            "com1",
            "lpt9.cfg",
            "a/con.txt",
            "x/COM1",
        ] {
            assert!(safe_join(base, evil).is_err(), "应拒绝: {evil}");
        }
    }

    #[test]
    fn rejects_control_chars_and_bad_suffixes() {
        let base = Path::new(BASE);
        assert!(safe_join(base, "a\u{01}b").is_err());
        // 尾随点/空格仅对中间组件生效（首尾空白会被 trim）
        assert!(safe_join(base, "x/ trail.").is_err());
        assert!(safe_join(base, "trail . gguf").is_ok());
    }

    #[test]
    fn result_always_stays_under_base() {
        let base = Path::new(BASE);
        let joined = safe_join(base, "a/b/c.gguf").unwrap();
        assert!(joined.starts_with(base));
    }
}
