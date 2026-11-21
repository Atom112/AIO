//! Windows 无障碍后端（UI Automation）。
//!
//! COM 线程约束：`UIAutomation`/`UIElement` 都是 COM 接口指针，**只在本线程内创建并
//! 用完**——每个入口各自在 `spawn_blocking` 里新建一次、用完即弃，绝不在线程之间传递。
//! `UIAutomation::new()` 会初始化 MTA；若该线程已经初始化过（`RPC_E_CHANGED_MODE`），
//! 回退到 `new_direct()` 复用已存在的套间。
//!
//! 提权窗口：非提权进程向提权窗口注入输入会被 UIPI 静默丢弃（返回成功但无效果）。
//! 这里不做检测——检测本身不可靠，且判定结果无法改变行为；需要时可让用户提权运行 AIO。
//!
//! 坐标空间：UIA 的 BoundingRectangle 与 xcap/enigo 都是**屏幕物理像素**（实测：
//! per-monitor-v2 aware 进程下 UIA 窗口矩形与 xcap 窗口矩形一致）。前提是进程本身
//! DPI aware——tao/AIO 主进程已设置；未设置清单的测试进程会拿到系统虚拟化后的逻辑
//! 坐标（约 1/1.5），那不是坐标系 bug，而是缺清单。

use uiautomation::core::UIAutomation;
use uiautomation::patterns::{UIInvokePattern, UIValuePattern, UIWindowPattern};
use uiautomation::types::{Handle, Point, WindowVisualState};
use uiautomation::UIElement;

use super::{cap, normalize_role, TreeNode, MAX_DEPTH, REF_SCAN_CAP};
use crate::ierr;
use crate::utils::computer::{stale_ref_error, Rect, RefIdent, RefTarget, UiNode, UiWindow};

/// 建立一次 UIA 客户端（每个阻塞调用一次）。
fn automation() -> Result<UIAutomation, String> {
    UIAutomation::new()
        .or_else(|_| UIAutomation::new_direct())
        .map_err(|e| {
            ierr!(
                "初始化 UI Automation 失败（请重启 AIO 后重试）: {}",
                "Failed to initialize UI Automation (restart AIO and retry): {}",
                e
            )
        })
}

/// UIA 矩形 → 统一矩形。
fn rect_of(element: &UIElement) -> Option<Rect> {
    let rect = element.get_bounding_rectangle().ok()?;
    Some(Rect {
        x: rect.get_left(),
        y: rect.get_top(),
        width: rect.get_right() - rect.get_left(),
        height: rect.get_bottom() - rect.get_top(),
    })
}

/// 定位窗口根元素：先按 HWND，再用进程 ID + 标题在控件视图里找。
fn window_element(automation: &UIAutomation, window: &UiWindow) -> Result<UIElement, String> {
    if let Ok(element) = automation.element_from_handle(Handle::from(window.id as isize)) {
        return Ok(element);
    }
    let walker = automation
        .get_control_view_walker()
        .map_err(|e| e.to_string())?;
    let root = automation.get_root_element().map_err(|e| e.to_string())?;
    let mut stack: Vec<UIElement> = vec![root];
    let mut visited = 0usize;
    while let Some(element) = stack.pop() {
        visited += 1;
        if visited > REF_SCAN_CAP {
            break;
        }
        let pid_matches = element.get_process_id().ok() == Some(window.pid);
        if pid_matches {
            let name = element.get_name().unwrap_or_default();
            if name == window.title || (!window.title.is_empty() && name.contains(&window.title)) {
                return Ok(element);
            }
        }
        if let Some(children) = walker.get_children(&element) {
            stack.extend(children);
        }
    }
    Err(ierr!(
        "没有找到窗口 \"{}\" 的无障碍根元素（窗口可能已关闭）",
        "No accessibility root found for window \"{}\" (it may have closed)",
        window.title
    ))
}

/// 采集一个元素的节点数据（逐属性容错：单个 getter 失败不毁掉整棵树）。
fn node_of(element: &UIElement, depth: usize) -> UiNode {
    let role = element
        .get_control_type()
        .map(|kind| normalize_role(&format!("{kind:?}")))
        .unwrap_or_else(|_| "unknown".to_string());
    let name = cap(&element.get_name().unwrap_or_default(), 200);
    let ident = element
        .get_automation_id()
        .ok()
        .filter(|value| !value.is_empty());
    let value = element
        .get_pattern::<UIValuePattern>()
        .ok()
        .and_then(|pattern| pattern.get_value().ok())
        .filter(|text| !text.is_empty());
    UiNode {
        depth,
        role,
        name,
        value,
        ident,
        rect: rect_of(element).unwrap_or(super::EMPTY_RECT),
        enabled: element.is_enabled().unwrap_or(true),
    }
}

/// 遍历目标窗口的无障碍树（前序，深度与节点数双重上限）。
fn window_tree_blocking(window: &UiWindow, max_nodes: usize) -> Result<Vec<TreeNode>, String> {
    let automation = automation()?;
    let root = window_element(&automation, window)?;
    let walker = automation
        .get_control_view_walker()
        .map_err(|e| e.to_string())?;
    let mut out: Vec<TreeNode> = Vec::with_capacity(max_nodes.min(256));
    let mut stack: Vec<(UIElement, usize)> = vec![(root, 0)];
    while let Some((element, depth)) = stack.pop() {
        if out.len() >= max_nodes {
            break;
        }
        // 离屏子树整体跳过：既不可见也无法点击
        if depth > 0 && element.is_offscreen().unwrap_or(false) {
            continue;
        }
        let runtime_id = element.get_runtime_id().unwrap_or_default();
        out.push(TreeNode {
            node: node_of(&element, depth),
            ident: RefIdent::Windows { runtime_id },
        });
        if depth >= MAX_DEPTH {
            continue;
        }
        if let Some(children) = walker.get_children(&element) {
            // 栈是后进先出，反序压入保持视觉顺序
            for child in children.into_iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }
    Ok(out)
}

/// 遍历目标窗口的无障碍树（异步入口）。
pub(crate) async fn window_tree(
    window: &UiWindow,
    max_nodes: usize,
) -> Result<Vec<TreeNode>, String> {
    let window = window.clone();
    tokio::task::spawn_blocking(move || window_tree_blocking(&window, max_nodes))
        .await
        .map_err(|e| e.to_string())?
}

/// 聚焦窗口（最小化时先还原）。
pub(crate) async fn focus_window(window: &UiWindow) -> Result<(), String> {
    let window = window.clone();
    tokio::task::spawn_blocking(move || {
        let automation = automation()?;
        let element = window_element(&automation, &window)?;
        if let Ok(pattern) = element.get_pattern::<UIWindowPattern>() {
            // 还原失败不影响后续聚焦（有些窗口不支持 WindowPattern）
            let _ = pattern.set_window_visual_state(WindowVisualState::Normal);
        }
        element
            .set_focus()
            .map_err(|e| ierr!("聚焦窗口失败: {}", "Failed to focus the window: {}", e))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 重新定位元素：runtime id 优先（跨进程稳定），再按点定位，最后按标识+角色+名称。
fn resolve_element(automation: &UIAutomation, target: &RefTarget) -> Result<UIElement, String> {
    let RefIdent::Windows { runtime_id } = &target.ident else {
        return Err(stale_ref_error(&target.ref_label));
    };
    let same = |element: &UIElement| {
        element.get_runtime_id().ok().as_deref() == Some(runtime_id.as_slice())
    };

    let (cx, cy) = target.rect.center();
    if let Ok(element) = automation.element_from_point(Point::new(cx, cy)) {
        if same(&element) {
            return Ok(element);
        }
    }

    let root = window_element(automation, &target.window)?;
    let walker = automation
        .get_control_view_walker()
        .map_err(|e| e.to_string())?;
    let mut stack: Vec<UIElement> = vec![root];
    let mut visited = 0usize;
    let mut fallback: Option<UIElement> = None;
    while let Some(element) = stack.pop() {
        visited += 1;
        if visited > REF_SCAN_CAP {
            break;
        }
        if same(&element) {
            return Ok(element);
        }
        if fallback.is_none() {
            // 名称 + 角色匹配作为兜底键；名称为空时再用自动化 ID 兜底
            // （自动化 ID 在部分框架里为空，不强制）。
            let role_matches = element
                .get_control_type()
                .map(|kind| normalize_role(&format!("{kind:?}")) == target.role)
                .unwrap_or(false);
            let name = element.get_name().unwrap_or_default();
            let ident_matches = target.ident_hint.as_deref().is_some_and(|hint| {
                element
                    .get_automation_id()
                    .map(|actual| actual == hint)
                    .unwrap_or(false)
            });
            if role_matches && (name == target.name || (target.name.is_empty() && ident_matches)) {
                fallback = Some(element.clone());
            }
        }
        if let Some(children) = walker.get_children(&element) {
            stack.extend(children);
        }
    }
    fallback.ok_or_else(|| stale_ref_error(&target.ref_label))
}

/// 原生激活（Invoke 模式）；无该模式时返回 `Ok(false)` 让调用方物理点击。
pub(crate) async fn press(target: &RefTarget) -> Result<bool, String> {
    let target = target.clone();
    tokio::task::spawn_blocking(move || {
        let automation = automation()?;
        let element = resolve_element(&automation, &target)?;
        match element.get_pattern::<UIInvokePattern>() {
            Ok(pattern) => Ok(pattern.invoke().is_ok()),
            Err(_) => Ok(false),
        }
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 聚焦元素（set_focus）。
pub(crate) async fn focus_element(target: &RefTarget) -> Result<(), String> {
    let target = target.clone();
    tokio::task::spawn_blocking(move || {
        let automation = automation()?;
        let element = resolve_element(&automation, &target)?;
        element
            .set_focus()
            .map_err(|e| ierr!("聚焦元素失败: {}", "Failed to focus the element: {}", e))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 无障碍文本插入：聚焦元素后用剪贴板粘贴（`Ctrl+V`），剪贴板内容会尽力快照并还原。
///
/// 为什么不用 `ValuePattern::SetValue`：那是**整体替换**，会抹掉字段里已有内容；粘贴是
/// 插入语义，且与 IME 无关（物理打字在中文输入法激活时会被组合改写，实测整段文本被吞）。
/// 元素不支持粘贴或剪贴板不可用时返回 `Ok(false)`，调用方回退物理打字。
pub(crate) async fn insert_text(target: &RefTarget, text: &str) -> Result<bool, String> {
    let target = target.clone();
    let text = text.to_string();
    tokio::task::spawn_blocking(move || {
        let automation = automation()?;
        let element = resolve_element(&automation, &target)?;
        Ok(element.send_text_by_clipboard(&text).is_ok())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_normalization_strips_case() {
        assert_eq!(normalize_role("Button"), "button");
        assert_eq!(normalize_role("ListItem"), "listitem");
        assert_eq!(normalize_role("  "), "unknown");
    }
}
