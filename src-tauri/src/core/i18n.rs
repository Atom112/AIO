//! 后端用户可见文案的 locale 支持。
//!
//! 此前 Tauri 命令返回的错误与 Agent 工具错误全部硬编码中文。
//! 本模块提供：
//! - 全局 locale（前端启动/切换语言时经 `set_locale` 命令同步；缺省 zh-CN）；
//! - [`ierr!`] 宏：就地双语模板，按当前 locale 选择其一。
//!
//! 迁移约定：
//! - 仅迁移**用户可见**的错误（命令层 `Result<_, String>` / 工具错误 / 事件错误载荷）；
//! - tracing 日志、内部错误链上未冒泡到用户面的中间层保持原样；
//! - `map_err(|e| format!("中文: {e}"))` → `map_err(|e| ierr!("中文: {}", "English: {}", e))`
//!   （占位符必须用位置 `{}`，禁用内联捕获 `{e}`——与显式参数混用会触发 redundant-argument 警告；
//!   底层库错误原文作为参数插入，保持技术细节可诊断）。

use std::sync::{OnceLock, RwLock};

static LOCALE: OnceLock<RwLock<String>> = OnceLock::new();

fn locale_cell() -> &'static RwLock<String> {
    LOCALE.get_or_init(|| RwLock::new("zh-CN".to_string()))
}

/// 当前 locale（"zh-CN" / "en-US"；未知值按中文处理）。
pub(crate) fn current_locale() -> String {
    locale_cell()
        .read()
        .map(|g| g.clone())
        .unwrap_or_else(|_| "zh-CN".to_string())
}

/// 设置全局 locale（`set_locale` 命令调用）。
pub(crate) fn set_locale(locale: String) {
    let normalized = match locale.as_str() {
        "en-US" => "en-US".to_string(),
        _ => "zh-CN".to_string(),
    };
    let _ = locale_cell().write().map(|mut g| *g = normalized);
}

/// 当前是否英文（en-US）。
pub(crate) fn is_en() -> bool {
    current_locale() == "en-US"
}

/// 在中文/英文模板间选择（无参数场景，返回 String）。
pub(crate) fn pick(zh: &str, en: &str) -> String {
    if is_en() {
        en.to_string()
    } else {
        zh.to_string()
    }
}

/// 就地双语错误消息宏：按当前 locale 选择模板。
///
/// 两种形态：
/// - `ierr!("中文", "English")` —— 静态文案，返回 `String`；
/// - `ierr!("中文 {e}", "English {e}", e)` —— `format!` 参数（支持内联捕获 `{e}`）。
///
/// 示例：
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
        // 恢复默认，避免影响其他测试
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
