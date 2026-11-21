//! 平台无障碍后端接口（UIA / AXUIElement / AT-SPI2）。
//!
//! 用自由函数而不是 `dyn` trait：Linux 后端必须 async（zbus），Windows/macOS 必须在
//! 阻塞池里跑 COM/AX 调用——两种形态塞进同一 trait 只会让每个调用点多一层包装。
//!
//! 契约：
//! - `window_tree`：返回目标窗口的无障碍子树（含窗口根节点，`depth` 从 0 起），
//!   绝对屏幕物理像素坐标，深度上限 12、节点数上限由调用方给定；
//! - `press` / `insert_text` 返回 `Ok(false)` 表示「该元素没有对应原生接口」，
//!   调用方回退到物理输入——不是错误；
//! - `focus_window` / `focus_element`：尽力而为的聚焦（不改窗口尺寸/层级）；
//! - 元素已消失时返回共享的 stale-ref 文案，引导模型重新快照。

use super::{Rect, RefIdent, UiNode};
use crate::utils::computer::truncate_chars;

/// 元素树深度上限（再深的层级对定位没有价值，且会放大遍历成本）。
pub(crate) const MAX_DEPTH: usize = 12;

/// 重新定位元素时的最大扫描节点数。
pub(crate) const REF_SCAN_CAP: usize = 5_000;

/// 一个树节点 = 展示数据 + 平台原生身份。
pub(crate) struct TreeNode {
    /// 快照文本行数据
    pub(crate) node: UiNode,
    /// 平台原生身份（引用重新定位用）
    pub(crate) ident: RefIdent,
}

/// 角色名归一化：小写 + 去掉 macOS 的 `AX` 前缀。
pub(crate) fn normalize_role(raw: &str) -> String {
    let lower = raw.trim().to_lowercase();
    let stripped = lower.strip_prefix("ax").unwrap_or(&lower);
    if stripped.is_empty() {
        "unknown".to_string()
    } else {
        stripped.to_string()
    }
}

/// 名称/值的统一截断（后端不负责最终排版，只防止把超长文本带进快照）。
pub(crate) fn cap(text: &str, max: usize) -> String {
    truncate_chars(text, max)
}

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::*;

/// 非三平台的兜底实现（移动端等）：明确报不支持，不做假动作。
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
mod unsupported;
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
pub(crate) use unsupported::*;

/// 空矩形（接口缺失时的占位，快照里显示 `rect=0,0,0,0`）。
pub(crate) const EMPTY_RECT: Rect = Rect {
    x: 0,
    y: 0,
    width: 0,
    height: 0,
};
