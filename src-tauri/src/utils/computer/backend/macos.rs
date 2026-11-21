//! macOS 无障碍后端（AXUIElement）。
//!
//! 权限：读取其他应用的无障碍树需要「辅助功能」授权。未授权时 `AX*` 调用全部返回
//! `kAXErrorAPIDisabled`，没有任何部分可用状态——所以进入每个入口先做一次
//! `AXIsProcessTrustedWithOptions({prompt:true})`，未授权直接给出系统设置路径。
//! 屏幕录制是另一项独立授权（截图用，见 `capture`）。
//!
//! 线程：`AXUIElement` 是 CFType 句柄（Send+Sync），跨进程属性读取是同步 IPC，
//! 因此整棵树的遍历放在阻塞池里跑，绝不占用 async worker。

use axuielement::ax_action::{AX_PRESS_ACTION, AX_RAISE_ACTION};
use axuielement::ax_attribute::attributes as attrs;
use axuielement::{is_process_trusted_with_prompt, AXUIElement};

use super::{cap, normalize_role, TreeNode, MAX_DEPTH, REF_SCAN_CAP};
use crate::ierr;
use crate::utils::computer::{stale_ref_error, Rect, RefIdent, RefTarget, UiNode, UiWindow};

/// 单次属性读取的 IPC 超时（秒）——避免对方无响应时把整次快照挂住。
const AX_TIMEOUT_SECS: f32 = 1.5;

/// 坐标回退匹配的容差（目标元素位置小幅漂移仍视为同一元素）。
const MATCH_TOLERANCE: i32 = 40;

/// 辅助功能授权门。
fn ensure_trusted() -> Result<(), String> {
    if is_process_trusted_with_prompt() {
        return Ok(());
    }
    Err(ierr!(
        "需要辅助功能权限：系统设置 → 隐私与安全性 → 辅助功能，允许 AIO 后重试",
        "Accessibility permission required: System Settings → Privacy & Security → Accessibility, allow AIO and retry"
    ))
}

/// 应用根元素（带超时）。
fn app_root(window: &UiWindow) -> Result<AXUIElement, String> {
    let element = AXUIElement::from_pid(window.pid as i32).ok_or_else(|| {
        ierr!(
            "无法连接到应用进程 {}（窗口可能已关闭）",
            "Cannot attach to application process {} (the window may have closed)",
            window.pid
        )
    })?;
    let _ = element.set_timeout(AX_TIMEOUT_SECS);
    Ok(element)
}

/// 目标窗口元素：先按标题在 `AXWindows` 里找，再退回聚焦窗口，最后退回应用根。
fn window_root(app: &AXUIElement, window: &UiWindow) -> AXUIElement {
    if let Ok(windows) = app.element_array_attribute(attrs::AX_WINDOWS_ATTRIBUTE) {
        for candidate in windows {
            let title = candidate
                .string_attribute(attrs::AX_TITLE_ATTRIBUTE)
                .ok()
                .flatten();
            if title.as_deref() == Some(window.title.as_str()) {
                return candidate;
            }
        }
    }
    if let Ok(Some(focused)) = app.element_attribute(attrs::AX_FOCUSED_WINDOW_ATTRIBUTE) {
        return focused;
    }
    app.clone()
}

/// 元素矩形（位置 + 尺寸，四舍五入到整数像素）。
fn rect_of(element: &AXUIElement) -> Option<Rect> {
    let position = element
        .point_attribute(attrs::AX_POSITION_ATTRIBUTE)
        .ok()??;
    let size = element.size_attribute(attrs::AX_SIZE_ATTRIBUTE).ok()??;
    Some(Rect {
        x: position.x.round() as i32,
        y: position.y.round() as i32,
        width: size.width.round() as i32,
        height: size.height.round() as i32,
    })
}

/// 采集一个元素的节点数据（逐属性容错）。
fn node_of(element: &AXUIElement, depth: usize) -> UiNode {
    let role = element
        .string_attribute(attrs::AX_ROLE_ATTRIBUTE)
        .ok()
        .flatten()
        .map(|raw| normalize_role(&raw))
        .unwrap_or_else(|| "unknown".to_string());
    let name = element
        .string_attribute(attrs::AX_TITLE_ATTRIBUTE)
        .ok()
        .flatten()
        .or_else(|| {
            element
                .string_attribute(attrs::AX_DESCRIPTION_ATTRIBUTE)
                .ok()
                .flatten()
        })
        .unwrap_or_default();
    let value = element
        .string_attribute(attrs::AX_VALUE_ATTRIBUTE)
        .ok()
        .flatten()
        .filter(|text| !text.is_empty());
    let ident = element
        .string_attribute(attrs::AX_IDENTIFIER_ATTRIBUTE)
        .ok()
        .flatten()
        .filter(|text| !text.is_empty());
    UiNode {
        depth,
        role,
        name: cap(&name, 200),
        value,
        ident,
        rect: rect_of(element).unwrap_or(super::EMPTY_RECT),
        enabled: element
            .bool_attribute(attrs::AX_ENABLED_ATTRIBUTE)
            .ok()
            .flatten()
            .unwrap_or(true),
    }
}

/// 是否隐藏（`AXHidden`）。
fn is_hidden(element: &AXUIElement) -> bool {
    element
        .bool_attribute(attrs::AX_HIDDEN_ATTRIBUTE)
        .ok()
        .flatten()
        == Some(true)
}

/// 遍历窗口树（前序，depth/max_nodes 双上限；隐藏子树整体跳过）。
fn collect_tree(root: &AXUIElement, max_nodes: usize) -> Vec<TreeNode> {
    let mut out: Vec<TreeNode> = Vec::with_capacity(max_nodes.min(256));
    let mut stack: Vec<(AXUIElement, usize, Vec<usize>)> = vec![(root.clone(), 0, Vec::new())];
    while let Some((element, depth, path)) = stack.pop() {
        if out.len() >= max_nodes {
            break;
        }
        if depth > 0 && is_hidden(&element) {
            continue;
        }
        out.push(TreeNode {
            node: node_of(&element, depth),
            ident: RefIdent::Mac { path: path.clone() },
        });
        if depth >= MAX_DEPTH {
            continue;
        }
        if let Ok(children) = element.children() {
            for (index, child) in children.into_iter().enumerate().rev() {
                let mut child_path = path.clone();
                child_path.push(index);
                stack.push((child, depth + 1, child_path));
            }
        }
    }
    out
}

/// 快照入口（阻塞池）。
pub(crate) async fn window_tree(
    window: &UiWindow,
    max_nodes: usize,
) -> Result<Vec<TreeNode>, String> {
    let window = window.clone();
    tokio::task::spawn_blocking(move || {
        ensure_trusted()?;
        let app = app_root(&window)?;
        let root = window_root(&app, &window);
        Ok(collect_tree(&root, max_nodes))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 聚焦窗口（Raise + 设为主窗口）。
pub(crate) async fn focus_window(window: &UiWindow) -> Result<(), String> {
    let window = window.clone();
    tokio::task::spawn_blocking(move || {
        ensure_trusted()?;
        let app = app_root(&window)?;
        let root = window_root(&app, &window);
        root.perform_action(AX_RAISE_ACTION)
            .map_err(|e| ierr!("聚焦窗口失败: {}", "Failed to focus the window: {}", e))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 按索引路径取元素。
fn element_at_path(root: &AXUIElement, path: &[usize]) -> Option<AXUIElement> {
    let mut current = root.clone();
    for index in path {
        let children = current.children().ok()?;
        current = children.get(*index)?.clone();
    }
    Some(current)
}

/// 角色与名称是否仍然匹配。
fn identity_matches(element: &AXUIElement, target: &RefTarget) -> bool {
    let role = element
        .string_attribute(attrs::AX_ROLE_ATTRIBUTE)
        .ok()
        .flatten()
        .map(|raw| normalize_role(&raw))
        .unwrap_or_default();
    if role != target.role {
        return false;
    }
    let name = element
        .string_attribute(attrs::AX_TITLE_ATTRIBUTE)
        .ok()
        .flatten()
        .or_else(|| {
            element
                .string_attribute(attrs::AX_DESCRIPTION_ATTRIBUTE)
                .ok()
                .flatten()
        })
        .unwrap_or_default();
    cap(&name, 200) == target.name
}

/// 重新定位引用元素：路径优先（界面未变时最精确），失败再按角色+名称+位置兜底扫描。
fn resolve_element(window: &UiWindow, target: &RefTarget) -> Result<AXUIElement, String> {
    let RefIdent::Mac { path } = &target.ident else {
        return Err(stale_ref_error(&target.ref_label));
    };
    let app = app_root(window)?;
    let root = window_root(&app, window);
    if let Some(element) = element_at_path(&root, path) {
        if identity_matches(&element, target) {
            return Ok(element);
        }
    }

    let mut stack: Vec<AXUIElement> = vec![root];
    let mut visited = 0usize;
    while let Some(element) = stack.pop() {
        visited += 1;
        if visited > REF_SCAN_CAP {
            break;
        }
        if identity_matches(&element, target) {
            let (cx, cy) = target.rect.center();
            let near = rect_of(&element)
                .map(|rect| {
                    let (x, y) = rect.center();
                    (x - cx).abs() <= MATCH_TOLERANCE && (y - cy).abs() <= MATCH_TOLERANCE
                })
                .unwrap_or(false);
            if near {
                return Ok(element);
            }
        }
        if let Ok(children) = element.children() {
            stack.extend(children);
        }
    }
    Err(stale_ref_error(&target.ref_label))
}

/// 原生激活（AXPress）；没有该动作时返回 `Ok(false)`。
pub(crate) async fn press(target: &RefTarget) -> Result<bool, String> {
    let target = target.clone();
    tokio::task::spawn_blocking(move || {
        ensure_trusted()?;
        let element = resolve_element(&target.window, &target)?;
        let names = element.action_names().unwrap_or_default();
        if !names.iter().any(|name| name == AX_PRESS_ACTION) {
            return Ok(false);
        }
        Ok(element.perform_action(AX_PRESS_ACTION).is_ok())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 聚焦元素：先设 `AXFocused`，失败再用 `AXRaise`。
pub(crate) async fn focus_element(target: &RefTarget) -> Result<(), String> {
    let target = target.clone();
    tokio::task::spawn_blocking(move || {
        ensure_trusted()?;
        let element = resolve_element(&target.window, &target)?;
        if element
            .set_bool_attribute(attrs::AX_FOCUSED_ATTRIBUTE, true)
            .is_ok()
        {
            return Ok(());
        }
        element
            .perform_action(AX_RAISE_ACTION)
            .map_err(|e| ierr!("聚焦元素失败: {}", "Failed to focus the element: {}", e))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// macOS 上物理打字更保真，故一律返回 `Ok(false)`。
pub(crate) async fn insert_text(_target: &RefTarget, _text: &str) -> Result<bool, String> {
    Ok(false)
}
