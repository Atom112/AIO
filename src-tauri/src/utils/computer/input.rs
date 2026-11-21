//! 物理输入注入（鼠标/键盘）——enigo 的薄封装。
//!
//! 边界：只做「把事件送进当前桌面会话」，不判断元素、不做权限（权限在
//! `core::permission`，元素定位在 `backend`）。每个入口都在阻塞池里现建
//! `Enigo`（该类型与线程 STW 绑定，不能跨 await 复用），并保留可取消的
//! 拖拽路径。Linux 上没有 X11/XWayland 会话时显式报错，绝不静默 no-op。

use std::time::Duration;

use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use tokio_util::sync::CancellationToken;

use crate::ierr;

/// 一次点击的鼠标键。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ButtonKind {
    /// 左键
    Left,
    /// 右键
    Right,
}

/// 解析后的组合键：修饰键按顺序按下、主键单击、修饰键反序释放。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeyCombo {
    /// 修饰键（按下顺序即此顺序）
    pub modifiers: Vec<Key>,
    /// 主键
    pub key: Key,
    /// 规范化后的可读标签（如 `ctrl+shift+s`），用于结果文案
    pub label: String,
}

/// 输入注入失败的统一文案。
fn input_err(e: impl std::fmt::Display) -> String {
    ierr!("输入注入失败: {}", "Input injection failed: {}", e)
}

/// 建一个 `Enigo` 实例（调用方负责放进阻塞线程；Linux 先看 DISPLAY）。
fn new_enigo() -> Result<Enigo, String> {
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_none() {
        return Err(ierr!(
            "当前会话没有 X11/XWayland DISPLAY，无法注入鼠标/键盘；可改用 ref 动作（无障碍接口）或切换到 X11 会话",
            "No X11/XWayland DISPLAY in this session; mouse/keyboard injection is unavailable. Use ref actions (accessibility interfaces) or an X11 session"
        ));
    }
    Enigo::new(&Settings::default()).map_err(input_err)
}

/// 把 `ButtonKind` 映射到 enigo 的鼠标键。
fn map_button(button: ButtonKind) -> Button {
    match button {
        ButtonKind::Left => Button::Left,
        ButtonKind::Right => Button::Right,
    }
}

/// 单击/双击（`count` = 1 或 2），60 ms 间隔。
pub(crate) async fn click(x: i32, y: i32, button: ButtonKind, count: u8) -> Result<(), String> {
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut enigo = new_enigo()?;
        enigo.move_mouse(x, y, Coordinate::Abs).map_err(input_err)?;
        let button = map_button(button);
        for index in 0..count.max(1) {
            if index > 0 {
                std::thread::sleep(Duration::from_millis(60));
            }
            enigo.button(button, Direction::Click).map_err(input_err)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 移动光标到绝对坐标。
pub(crate) async fn move_to(x: i32, y: i32) -> Result<(), String> {
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut enigo = new_enigo()?;
        enigo.move_mouse(x, y, Coordinate::Abs).map_err(input_err)?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 拖拽：按下 → 10 步插值移动（15 ms/步）→ 释放；每步检查取消。
pub(crate) async fn drag(
    from: (i32, i32),
    to: (i32, i32),
    token: &CancellationToken,
) -> Result<(), String> {
    let token = token.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut enigo = new_enigo()?;
        enigo
            .move_mouse(from.0, from.1, Coordinate::Abs)
            .map_err(input_err)?;
        enigo
            .button(Button::Left, Direction::Press)
            .map_err(input_err)?;
        let release = |enigo: &mut Enigo| -> Result<(), String> {
            enigo
                .button(Button::Left, Direction::Release)
                .map_err(input_err)
        };
        for step in 1..=10 {
            if token.is_cancelled() {
                let _ = release(&mut enigo);
                return Err(ierr!("操作已取消", "cancelled"));
            }
            let x = from.0 + (to.0 - from.0) * step / 10;
            let y = from.1 + (to.1 - from.1) * step / 10;
            if let Err(e) = enigo.move_mouse(x, y, Coordinate::Abs) {
                let _ = release(&mut enigo);
                return Err(input_err(e));
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        release(&mut enigo)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 滚轮滚动：正数向上（enigo 的语义是正数向下，这里统一取反）。
pub(crate) async fn scroll(x: i32, y: i32, amount: i32) -> Result<(), String> {
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut enigo = new_enigo()?;
        enigo.move_mouse(x, y, Coordinate::Abs).map_err(input_err)?;
        enigo.scroll(-amount, Axis::Vertical).map_err(input_err)?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 输入文本（Unicode 键事件）。
///
/// 物理打字会经过当前输入法：IME 处于中文/日文等组合状态时，注入的字符可能被
/// 输入法改写（实测 Win11 记事本 + 微软拼音会把整段文本吞成一个字符）。所以
/// 「有元素引用」的输入应优先走无障碍路径（Windows 是剪贴板粘贴，Linux 是
/// EditableText），本函数只作为没有引用的兜底。
pub(crate) async fn type_text(text: &str) -> Result<(), String> {
    let text = text.to_string();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut enigo = new_enigo()?;
        enigo.text(&text).map_err(input_err)?;
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 发送组合键：修饰键依次按下 → 主键单击 → 修饰键反序释放（30 ms 间隔）。
///
/// 任何一步失败都先把已按下的修饰键释放，避免留下粘滞修饰键。
pub(crate) async fn press_keys(combo: &KeyCombo) -> Result<(), String> {
    let combo = combo.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        let mut enigo = new_enigo()?;
        let mut pressed = 0usize;
        let release_all = |enigo: &mut Enigo, pressed: usize| {
            for modifier in combo.modifiers[..pressed].iter().rev() {
                let _ = enigo.key(*modifier, Direction::Release);
            }
        };
        for modifier in &combo.modifiers {
            if let Err(e) = enigo.key(*modifier, Direction::Press) {
                release_all(&mut enigo, pressed);
                return Err(input_err(e));
            }
            pressed += 1;
            std::thread::sleep(Duration::from_millis(30));
        }
        let result = enigo.key(combo.key, Direction::Click);
        release_all(&mut enigo, pressed);
        result.map_err(input_err)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 当前光标位置（`scroll` 未给落点时使用）。
pub(crate) async fn cursor_position() -> Result<(i32, i32), String> {
    tokio::task::spawn_blocking(move || -> Result<(i32, i32), String> {
        let enigo = new_enigo()?;
        enigo.location().map_err(input_err)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// 组合键的可读标签（结果文案用）。
pub(crate) fn combo_label(combo: &KeyCombo) -> String {
    combo.label.clone()
}

/// 解析 `ctrl+shift+s` / `alt+tab` / `enter` / `f12` 形式的组合键。
///
/// 最后一个 token 是主键，之前的 token 必须是修饰键；
/// 单字符 token 映射为 `Key::Unicode`（保留大小写）。
/// enigo 只提供 F1–F20，`f21`–`f24` 会报未知按键。
pub(crate) fn parse_key_combo(spec: &str) -> Result<KeyCombo, String> {
    let unknown = |token: &str| ierr!("未知按键: {}", "Unknown key: {}", token);
    let tokens: Vec<&str> = spec.split('+').map(str::trim).collect();
    if tokens.is_empty() || tokens.iter().any(|t| t.is_empty()) {
        return Err(ierr!("未知按键: {}", "Unknown key: {}", spec));
    }
    let (key_token, modifier_tokens) = tokens.split_last().expect("非空 token 列表");
    let mut modifiers: Vec<Key> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    for token in modifier_tokens {
        let lower = token.to_lowercase();
        let modifier = match lower.as_str() {
            "ctrl" | "control" => Key::LControl,
            "shift" => Key::Shift,
            "alt" | "option" => Key::Alt,
            "meta" | "win" | "windows" | "super" | "cmd" | "command" => Key::Meta,
            _ => return Err(ierr!("未知修饰键: {}", "Unknown modifier: {}", token)),
        };
        if modifiers.contains(&modifier) {
            return Err(ierr!("修饰键重复: {}", "Duplicate modifier: {}", token));
        }
        modifiers.push(modifier);
        labels.push(lower);
    }

    let lower_key = key_token.to_lowercase();
    let key = match lower_key.as_str() {
        "enter" | "return" => Key::Return,
        "esc" | "escape" => Key::Escape,
        "tab" => Key::Tab,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "insert" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "up" | "arrowup" => Key::UpArrow,
        "down" | "arrowdown" => Key::DownArrow,
        "left" | "arrowleft" => Key::LeftArrow,
        "right" | "arrowright" => Key::RightArrow,
        "capslock" => Key::CapsLock,
        "printscreen" => Key::PrintScr,
        other => {
            if let Some(function) = parse_function_key(other) {
                function
            } else {
                let mut chars = key_token.chars();
                match (chars.next(), chars.next()) {
                    (Some(ch), None) => Key::Unicode(ch),
                    _ => return Err(unknown(key_token)),
                }
            }
        }
    };
    labels.push(lower_key);
    Ok(KeyCombo {
        modifiers,
        key,
        label: labels.join("+"),
    })
}

/// `f1`–`f20` → `Key::F*`（enigo 无 F21–F24）。
fn parse_function_key(token: &str) -> Option<Key> {
    let digits = token.strip_prefix('f')?;
    let number: usize = digits.parse().ok()?;
    Some(match number {
        1 => Key::F1,
        2 => Key::F2,
        3 => Key::F3,
        4 => Key::F4,
        5 => Key::F5,
        6 => Key::F6,
        7 => Key::F7,
        8 => Key::F8,
        9 => Key::F9,
        10 => Key::F10,
        11 => Key::F11,
        12 => Key::F12,
        13 => Key::F13,
        14 => Key::F14,
        15 => Key::F15,
        16 => Key::F16,
        17 => Key::F17,
        18 => Key::F18,
        19 => Key::F19,
        20 => Key::F20,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ctrl_shift_s() {
        let combo = parse_key_combo("ctrl+shift+s").unwrap();
        assert_eq!(combo.modifiers, vec![Key::LControl, Key::Shift]);
        assert_eq!(combo.key, Key::Unicode('s'));
        assert_eq!(combo.label, "ctrl+shift+s");
    }

    #[test]
    fn parse_named_keys() {
        assert_eq!(parse_key_combo("enter").unwrap().key, Key::Return);
        assert_eq!(parse_key_combo("f12").unwrap().key, Key::F12);
        let tab = parse_key_combo("alt+tab").unwrap();
        assert_eq!(tab.modifiers, vec![Key::Alt]);
        assert_eq!(tab.key, Key::Tab);
        assert_eq!(parse_key_combo("pgdn").unwrap().key, Key::PageDown);
        assert_eq!(parse_key_combo("cmd+c").unwrap().modifiers, vec![Key::Meta]);
    }

    #[test]
    fn parse_rejects_unknown_key() {
        assert!(parse_key_combo("ctrl+nope").is_err());
        assert!(parse_key_combo("hyper+a").is_err());
        assert!(parse_key_combo("f21").is_err());
        assert!(parse_key_combo("ctrl+").is_err());
        assert!(parse_key_combo("").is_err());
    }

    #[test]
    fn parse_is_case_insensitive() {
        let upper = parse_key_combo("CTRL+ENTER").unwrap();
        let lower = parse_key_combo("ctrl+enter").unwrap();
        assert_eq!(upper.modifiers, lower.modifiers);
        assert_eq!(upper.key, lower.key);
        // 单字符主键保留大小写（大写字母由模型显式给出，配合 shift 时才按下 shift）
        assert_eq!(parse_key_combo("ctrl+A").unwrap().key, Key::Unicode('A'));
    }
}
