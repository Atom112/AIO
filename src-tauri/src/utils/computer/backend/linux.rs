//! Linux 无障碍后端（AT-SPI2 / zbus）。
//!
//! 前提：桌面无障碍必须开启（GNOME: `gsettings set org.gnome.desktop.interface
//! toolkit-accessibility true`），且 `at-spi-bus-launcher` 在运行；否则连总线都连不上。
//!
//! Wayland 限制：AT-SPI 的**动作与文本插入**走 D-Bus，原生 Wayland 客户端照样可用；
//! 但物理输入注入（enigo/x11rb）只能到达 X11/XWayland 客户端——`input` 会在
//! 没有 `DISPLAY` 时显式报错，绝不静默 no-op。

use std::collections::HashMap;

use atspi::proxy::accessible::AccessibleProxy;
use atspi::proxy::proxy_ext::ProxyExt;
use atspi::{AccessibilityConnection, CoordType, State};

use super::{cap, normalize_role, TreeNode, MAX_DEPTH, REF_SCAN_CAP};
use crate::ierr;
use crate::utils::computer::{stale_ref_error, Rect, RefIdent, RefTarget, UiNode, UiWindow};

/// 连 AT-SPI 总线（每次调用一次；zbus 连接开销远小于一次 UI 遍历）。
async fn connect() -> Result<AccessibilityConnection, String> {
    AccessibilityConnection::new().await.map_err(|e| {
        ierr!(
            "无法连接 AT-SPI 无障碍总线：请确认已启用桌面无障碍（GNOME: gsettings set org.gnome.desktop.interface toolkit-accessibility true），且 at-spi-bus-launcher 正在运行（{}）",
            "Cannot connect to the AT-SPI accessibility bus: enable desktop accessibility (GNOME: gsettings set org.gnome.desktop.interface toolkit-accessibility true) and ensure at-spi-bus-launcher is running ({})",
            e
        )
    })
}

/// 由总线唯一名 + 对象路径构造 `AccessibleProxy`。
async fn accessible<'a>(
    conn: &'a zbus::Connection,
    bus: &str,
    path: &str,
) -> Result<AccessibleProxy<'a>, String> {
    let destination = zbus::names::BusName::try_from(bus.to_string())
        .map_err(|e| ierr!("无效的总线名: {}", "Invalid bus name: {}", e))?;
    let object_path = zbus::zvariant::ObjectPath::try_from(path.to_string())
        .map_err(|e| ierr!("无效的对象路径: {}", "Invalid object path: {}", e))?;
    AccessibleProxy::builder(conn)
        .cache_properties(zbus::proxy::CacheProperties::No)
        .destination(destination)
        .map_err(|e| e.to_string())?
        .path(object_path)
        .map_err(|e| e.to_string())?
        .build()
        .await
        .map_err(|e| e.to_string())
}

/// 元素矩形：屏幕坐标系的 extents；接口缺失时为 0 矩形。
async fn rect_of(proxy: &AccessibleProxy<'_>) -> Rect {
    let Ok(proxies) = proxy.proxies().await else {
        return super::EMPTY_RECT;
    };
    let Ok(component) = proxies.component().await else {
        return super::EMPTY_RECT;
    };
    match component.get_extents(CoordType::Screen).await {
        Ok((x, y, width, height)) => Rect {
            x,
            y,
            width,
            height,
        },
        Err(_) => super::EMPTY_RECT,
    }
}

/// 值：优先 `valuetext`，其次 `value`（AT-SPI 把两者都放在属性表里）。
async fn value_of(proxy: &AccessibleProxy<'_>) -> Option<String> {
    let attributes: HashMap<String, String> = proxy.get_attributes().await.ok()?;
    attributes
        .get("valuetext")
        .or_else(|| attributes.get("value"))
        .map(|text| cap(text, 200))
        .filter(|text| !text.is_empty())
}

/// 是否处于启用状态。
async fn enabled_of(proxy: &AccessibleProxy<'_>) -> bool {
    proxy
        .get_state()
        .await
        .map(|state| state.contains(State::Enabled))
        .unwrap_or(true)
}

/// 采集一个元素的节点数据。
async fn node_of(proxy: &AccessibleProxy<'_>, depth: usize) -> UiNode {
    let role = proxy
        .get_role_name()
        .await
        .map(|raw| normalize_role(&raw))
        .unwrap_or_else(|_| "unknown".to_string());
    let name = cap(&proxy.name().await.unwrap_or_default(), 200);
    let ident = proxy
        .accessible_id()
        .await
        .ok()
        .filter(|value| !value.is_empty());
    UiNode {
        depth,
        role,
        name,
        value: value_of(proxy).await,
        ident,
        rect: rect_of(proxy).await,
        enabled: enabled_of(proxy).await,
    }
}

/// 遍历应用树找目标窗口（frame/window 且名称匹配标题）。
async fn find_window<'a>(
    conn: &'a AccessibilityConnection,
    app_bus: &str,
    app_path: &str,
    window: &UiWindow,
) -> Result<AccessibleProxy<'a>, String> {
    let zbus_conn = conn.connection();
    let root = accessible(zbus_conn, app_bus, app_path).await?;
    if window.title.is_empty() {
        return Ok(root);
    }
    let mut stack: Vec<AccessibleProxy<'a>> = vec![root];
    let mut visited = 0usize;
    while let Some(proxy) = stack.pop() {
        visited += 1;
        if visited > REF_SCAN_CAP {
            break;
        }
        let role = proxy
            .get_role_name()
            .await
            .map(|raw| normalize_role(&raw))
            .unwrap_or_default();
        let name = proxy.name().await.unwrap_or_default();
        if (role == "frame" || role == "window" || role == "dialog") && name == window.title {
            return Ok(proxy);
        }
        if let Ok(children) = proxy.get_children().await {
            for child in children {
                let Some(bus) = child.name_as_str() else {
                    continue;
                };
                let path = child.path_as_str();
                if let Ok(child_proxy) = accessible(zbus_conn, bus, path).await {
                    stack.push(child_proxy);
                }
            }
        }
    }
    Err(ierr!(
        "没有找到窗口 \"{}\" 的无障碍对象",
        "No accessibility object found for window \"{}\"",
        window.title
    ))
}

/// 定位窗口树根：先按应用名找应用对象，再在其子树里找窗口。
async fn window_root<'a>(
    conn: &'a AccessibilityConnection,
    window: &UiWindow,
) -> Result<AccessibleProxy<'a>, String> {
    let registry = conn
        .root_accessible_on_registry()
        .await
        .map_err(|e| e.to_string())?;
    let apps = registry.get_children().await.map_err(|e| e.to_string())?;
    let needle = window.app_name.to_lowercase();
    let mut names: Vec<String> = Vec::new();
    for app in apps {
        let Some(bus) = app.name_as_str() else {
            continue;
        };
        let path = app.path_as_str();
        let Ok(proxy) = accessible(conn.connection(), bus, path).await else {
            continue;
        };
        let name = proxy.name().await.unwrap_or_default();
        names.push(name.clone());
        if needle.is_empty() || name.to_lowercase().contains(&needle) {
            if let Ok(root) = find_window(conn, bus, path, window).await {
                return Ok(root);
            }
        }
    }
    names.truncate(8);
    Err(ierr!(
        "没有找到应用 \"{}\" 的无障碍对象（可用应用: {}）",
        "No accessibility object found for app \"{}\" (available apps: {})",
        window.app_name,
        names.join(", ")
    ))
}

/// 快照入口：遍历目标窗口的 AT-SPI 树。
pub(crate) async fn window_tree(
    window: &UiWindow,
    max_nodes: usize,
) -> Result<Vec<TreeNode>, String> {
    let conn = connect().await?;
    let zbus_conn = conn.connection();
    let root = window_root(&conn, window).await?;
    let mut out: Vec<TreeNode> = Vec::with_capacity(max_nodes.min(256));
    let mut stack: Vec<(AccessibleProxy<'_>, usize)> = vec![(root, 0)];
    while let Some((proxy, depth)) = stack.pop() {
        if out.len() >= max_nodes {
            break;
        }
        let bus = proxy.inner().destination().to_string();
        let path = proxy.inner().path().to_string();
        out.push(TreeNode {
            node: node_of(&proxy, depth).await,
            ident: RefIdent::Linux { bus, path },
        });
        if depth >= MAX_DEPTH {
            continue;
        }
        if let Ok(children) = proxy.get_children().await {
            let mut built: Vec<AccessibleProxy<'_>> = Vec::new();
            for child in children {
                let Some(child_bus) = child.name_as_str() else {
                    continue;
                };
                let child_path = child.path_as_str();
                if let Ok(child_proxy) = accessible(zbus_conn, child_bus, child_path).await {
                    built.push(child_proxy);
                }
            }
            // 反序压栈保持视觉顺序
            for child in built.into_iter().rev() {
                stack.push((child, depth + 1));
            }
        }
    }
    Ok(out)
}

/// 聚焦窗口：优先 `Component.GrabFocus`，缺失时退回动作列表。
pub(crate) async fn focus_window(window: &UiWindow) -> Result<(), String> {
    let conn = connect().await?;
    let proxy = window_root(&conn, window).await?;
    focus_proxy(&proxy).await
}

/// 聚焦一个 `AccessibleProxy`（Component.grab_focus → 动作兜底）。
async fn focus_proxy(proxy: &AccessibleProxy<'_>) -> Result<(), String> {
    if let Ok(proxies) = proxy.proxies().await {
        if let Ok(component) = proxies.component().await {
            if component.grab_focus().await.unwrap_or(false) {
                return Ok(());
            }
        }
    }
    if let Ok(proxies) = proxy.proxies().await {
        if let Ok(action) = proxies.action().await {
            if action.do_action(0).await.unwrap_or(false) {
                return Ok(());
            }
        }
    }
    Err(ierr!(
        "聚焦元素失败：该元素没有可用的 Component/Action 接口",
        "Failed to focus the element: no usable Component/Action interface"
    ))
}

/// 重新定位引用元素：按身份重建代理，失败再按角色+名称+位置扫描应用树。
async fn resolve_element<'a>(
    conn: &'a AccessibilityConnection,
    target: &RefTarget,
) -> Result<AccessibleProxy<'a>, String> {
    let RefIdent::Linux { bus, path } = &target.ident else {
        return Err(stale_ref_error(&target.ref_label));
    };
    let zbus_conn = conn.connection();
    if let Ok(proxy) = accessible(zbus_conn, bus, path).await {
        let role = proxy
            .get_role_name()
            .await
            .map(|raw| normalize_role(&raw))
            .unwrap_or_default();
        let name = cap(&proxy.name().await.unwrap_or_default(), 200);
        if role == target.role && name == target.name {
            return Ok(proxy);
        }
    }

    let root = window_root(conn, &target.window).await?;
    let mut stack: Vec<AccessibleProxy<'a>> = vec![root];
    let mut visited = 0usize;
    let (cx, cy) = target.rect.center();
    while let Some(proxy) = stack.pop() {
        visited += 1;
        if visited > REF_SCAN_CAP {
            break;
        }
        let role = proxy
            .get_role_name()
            .await
            .map(|raw| normalize_role(&raw))
            .unwrap_or_default();
        let name = cap(&proxy.name().await.unwrap_or_default(), 200);
        if role == target.role && name == target.name {
            let rect = rect_of(&proxy).await;
            let (x, y) = rect.center();
            if (x - cx).abs() <= 40 && (y - cy).abs() <= 40 {
                return Ok(proxy);
            }
        }
        if let Ok(children) = proxy.get_children().await {
            for child in children {
                let Some(child_bus) = child.name_as_str() else {
                    continue;
                };
                let child_path = child.path_as_str();
                if let Ok(child_proxy) = accessible(zbus_conn, child_bus, child_path).await {
                    stack.push(child_proxy);
                }
            }
        }
    }
    Err(stale_ref_error(&target.ref_label))
}

/// 原生激活动作（Click/Press/Activate 优先，其次索引 0）；无动作返回 `Ok(false)`。
pub(crate) async fn press(target: &RefTarget) -> Result<bool, String> {
    let conn = connect().await?;
    let proxy = resolve_element(&conn, target).await?;
    let Ok(proxies) = proxy.proxies().await else {
        return Ok(false);
    };
    let Ok(action) = proxies.action().await else {
        return Ok(false);
    };
    let Ok(actions) = action.get_actions().await else {
        return Ok(false);
    };
    if actions.is_empty() {
        return Ok(false);
    }
    let index = actions
        .iter()
        .position(|candidate| {
            let name = candidate.name.to_lowercase();
            name.contains("click") || name.contains("press") || name.contains("activate")
        })
        .unwrap_or(0);
    Ok(action.do_action(index as i32).await.unwrap_or(false))
}

/// 聚焦元素。
pub(crate) async fn focus_element(target: &RefTarget) -> Result<(), String> {
    let conn = connect().await?;
    let proxy = resolve_element(&conn, target).await?;
    focus_proxy(&proxy).await
}

/// 无障碍文本插入（EditableText；CJK 文本无需 IME）。
pub(crate) async fn insert_text(target: &RefTarget, text: &str) -> Result<bool, String> {
    let conn = connect().await?;
    let proxy = resolve_element(&conn, target).await?;
    let Ok(proxies) = proxy.proxies().await else {
        return Ok(false);
    };
    let Ok(editable) = proxies.editable_text().await else {
        return Ok(false);
    };
    let caret = match proxies.text().await {
        Ok(text_proxy) => text_proxy.caret_offset().await.unwrap_or(0),
        Err(_) => 0,
    };
    let length = text.chars().count() as i32;
    Ok(editable
        .insert_text(caret, text, length)
        .await
        .unwrap_or(false))
}
