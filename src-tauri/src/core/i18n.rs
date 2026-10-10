//! Locale support for backend user-facing messages.
//!
//! Previously, Tauri command errors and agent tool errors were all hardcoded Chinese.
//! This module provides:
//! - a global locale (synced by the frontend via the `set_locale` command on startup / language switch; defaults to zh-CN);
//! - the [`ierr!`] macro: in-place bilingual templates, picks one per the current locale.
//!
//! Migration conventions:
//! - migrate only **user-visible** errors (command-layer `Result<_, String>` / tool errors / event error payloads);
//! - tracing logs and intermediate layers whose errors never bubble up to users stay as-is;
//! - `map_err(|e| format!("<Chinese message>: {e}"))` -> `map_err(|e| ierr!("<Chinese message>: {}", "<English message>: {}", e))`
//!   (placeholders must be positional `{}`; inline captures `{e}` are forbidden - mixing them with explicit
//!   arguments triggers a redundant-argument warning; the raw underlying error is passed as an argument so details stay diagnosable).

use std::sync::{OnceLock, RwLock};

static LOCALE: OnceLock<RwLock<String>> = OnceLock::new();

fn locale_cell() -> &'static RwLock<String> {
    LOCALE.get_or_init(|| RwLock::new("zh-CN".to_string()))
}

/// Current locale ("zh-CN" / "en-US"; unknown values are treated as Chinese).
pub(crate) fn current_locale() -> String {
    locale_cell()
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "zh-CN".to_string())
}

/// Set the global locale (called by the `set_locale` command).
pub(crate) fn set_locale(locale: String) {
    let normalized = match locale.as_str() {
        "en-US" => "en-US".to_string(),
        _ => "zh-CN".to_string(),
    };
    let _ = locale_cell().write().map(|mut g| *g = normalized);
}

/// Whether the current locale is English (en-US).
pub(crate) fn is_en() -> bool {
    current_locale() == "en-US"
}

/// Pick between the Chinese/English templates (no-argument case, returns String).
pub(crate) fn pick(zh: &str, en: &str) -> String {
    if is_en() {
        en.to_string()
    } else {
        zh.to_string()
    }
}

/// In-place bilingual error-message macro: picks the template by current locale.
///
/// Two forms:
/// - `ierr!("<Chinese message>", "<English message>")` - static text, returns a `String`;
/// - `ierr!("<Chinese message> {e}", "<English message> {e}", e)` - `format!` arguments (inline captures `{e}` supported).
///
/// Examples:
/// ```ignore
/// return Err(ierr!("项目名称不能为空", "Project name must not be empty"));
/// .map_err(|e| ierr!("读取配置失败: {e}", "Failed to read config: {e}", e))?;
/// ```
#[macro_export]
macro_rules! ierr {
    ($zh:expr, $en:expr) => {
        $crate::core::i18n::pick($zh, $en)
    };
    ($zh:expr, $en:expr, $($arg:tt)+) => {
        if $crate::core::i18n::is_en() {
            format!($en, $($arg)+)
        } else {
            format!($zh, $($arg)+)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_locale_is_zh() {
        assert_eq!(current_locale(), "zh-CN");
        assert!(!is_en());
    }

    #[test]
    fn set_locale_normalizes_unknown_to_zh() {
        set_locale("fr-FR".to_string());
        assert_eq!(current_locale(), "zh-CN");
        set_locale("en-US".to_string());
        assert_eq!(current_locale(), "en-US");
        // Restore the default so other tests are unaffected
        set_locale("zh-CN".to_string());
    }

    #[test]
    fn pick_follows_locale() {
        set_locale("en-US".to_string());
        assert_eq!(pick("中文", "English"), "English");
        set_locale("zh-CN".to_string());
        assert_eq!(pick("中文", "English"), "中文");
    }

    #[test]
    fn macro_formats_args_per_locale() {
        set_locale("en-US".to_string());
        assert_eq!(
            ierr!(
                "下载 {} 失败: {}",
                "Failed to download {}: {}",
                "a.bin",
                "boom"
            ),
            "Failed to download a.bin: boom"
        );
        set_locale("zh-CN".to_string());
        assert_eq!(
            ierr!(
                "下载 {} 失败: {}",
                "Failed to download {}: {}",
                "a.bin",
                "boom"
            ),
            "下载 a.bin 失败: boom"
        );
    }
}
