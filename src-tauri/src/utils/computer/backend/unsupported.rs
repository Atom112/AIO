//! 非桌面平台兜底后端：明确报不支持，绝不假装成功。
//!
//! 电脑控制依赖 Windows UIA / macOS AXUIElement / Linux AT-SPI2；其余平台（移动端等）
//! 既没有树也没有可靠输入注入，工具调用直接失败并说明原因。

use super::TreeNode;
use crate::ierr;
use crate::utils::computer::{RefTarget, UiWindow};

/// 统一的「本平台不支持」错误。
fn unsupported() -> String {
    ierr!(
        "当前平台不支持电脑控制（仅支持 Windows / macOS / Linux）",
        "Computer control is not supported on this platform (Windows / macOS / Linux only)"
    )
}

/// 快照：不支持。
pub(crate) async fn window_tree(
    _window: &UiWindow,
    _max_nodes: usize,
) -> Result<Vec<TreeNode>, String> {
    Err(unsupported())
}

/// 聚焦窗口：不支持。
pub(crate) async fn focus_window(_window: &UiWindow) -> Result<(), String> {
    Err(unsupported())
}

/// 原生激活：不支持（回退物理输入同样不可用）。
pub(crate) async fn press(_target: &RefTarget) -> Result<bool, String> {
    Err(unsupported())
}

/// 聚焦元素：不支持。
pub(crate) async fn focus_element(_target: &RefTarget) -> Result<(), String> {
    Err(unsupported())
}

/// 文本插入：不支持。
pub(crate) async fn insert_text(_target: &RefTarget, _text: &str) -> Result<bool, String> {
    Err(unsupported())
}
