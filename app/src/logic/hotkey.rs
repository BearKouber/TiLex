//! 全局快捷键管理：截图翻译。
//!
//! 全项目只有这一个全局快捷键，所以换键时直接注销上一个再注册新的，
//! 不需要按名称管理多个快捷键。

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use global_hotkey::hotkey::HotKey;
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};

use crate::error::Error;

static CURRENT_ID: AtomicU32 = AtomicU32::new(0);

thread_local! {
    static MANAGER: RefCell<Option<GlobalHotKeyManager>> = const { RefCell::new(None) };
    static CURRENT_HOTKEY: RefCell<Option<HotKey>> = const { RefCell::new(None) };
}

/// Slint 的一次按键翻译成 "Ctrl+Shift+A" 这种写法。旧版 `readHotkey`。
/// - `Some(s)`：录完了，`s` 是结果；`s == ""` 表示「清空」（按了退格）。
/// - `None`：还没录完 —— 只按到修饰键，或者这个键不支持。
pub fn accelerator(text: &str, ctrl: bool, shift: bool, alt: bool, meta: bool) -> Option<String> {
    if text.is_empty() {
        return None;
    }

    // 退格 = 清空
    if text == "\u{0008}" {
        return Some(String::new());
    }

    // Esc 不在这个函数里处理，界面层拦截
    if text == "\u{001b}" {
        return None;
    }

    // 修饰键本身：返回 None（继续等主键）
    match text {
        "\u{0010}" | "\u{0011}" | "\u{0012}" | "\u{0013}" | "\u{0014}" | "\u{0015}"
        | "\u{0016}" | "\u{0017}" | "\u{0018}" => return None,
        _ => {}
    }

    let key_name = match text {
        "\u{0009}" => "Tab",
        "\u{000a}" => "Enter",
        "\u{007f}" => "Delete",
        " " => "Space",
        "\u{F700}" => "Up",
        "\u{F701}" => "Down",
        "\u{F702}" => "Left",
        "\u{F703}" => "Right",
        "\u{F727}" => "Insert",
        "\u{F729}" => "Home",
        "\u{F72B}" => "End",
        "\u{F72C}" => "PageUp",
        "\u{F72D}" => "PageDown",
        "\u{F72F}" => "ScrollLock",
        "\u{F730}" => "Pause",
        "\u{F731}" => "PrintScreen",
        "`" => "`",
        "\\" => "\\",
        "[" => "[",
        "]" => "]",
        "," => ",",
        "=" => "=",
        "-" => "-",
        "." => ".",
        "'" => "'",
        ";" => ";",
        "/" => "/",
        _ => {
            let mut chars = text.chars();
            if let Some(c) = chars.next()
                && chars.next().is_none()
            {
                // F1 - F24: '\u{F704}' - '\u{F71B}'
                if ('\u{F704}'..='\u{F71B}').contains(&c) {
                    let num = (c as u32 - 0xF704) + 1;
                    return Some(format_hotkey(&format!("F{num}"), ctrl, shift, alt, meta));
                }
                if c.is_ascii_alphabetic() {
                    return Some(format_hotkey(
                        &c.to_ascii_uppercase().to_string(),
                        ctrl,
                        shift,
                        alt,
                        meta,
                    ));
                }
                if c.is_ascii_digit() {
                    return Some(format_hotkey(text, ctrl, shift, alt, meta));
                }
            }
            return None;
        }
    };

    Some(format_hotkey(key_name, ctrl, shift, alt, meta))
}

fn format_hotkey(key: &str, ctrl: bool, shift: bool, alt: bool, meta: bool) -> String {
    let mods = modifiers_only(ctrl, shift, alt, meta);
    if mods.is_empty() {
        key.to_string()
    } else {
        format!("{mods}+{key}")
    }
}

/// 只按着修饰键时给界面看的半成品，例如 "Ctrl+Shift"。没有修饰键就是空串。
pub fn modifiers_only(ctrl: bool, shift: bool, alt: bool, meta: bool) -> String {
    let mut parts = Vec::new();
    if ctrl {
        parts.push("Ctrl");
    }
    if shift {
        parts.push("Shift");
    }
    if meta {
        if std::env::consts::OS == "macos" {
            parts.push("Command");
        } else {
            parts.push("Super");
        }
    }
    if alt {
        parts.push("Alt");
    }
    parts.join("+")
}

/// 启动时调一次：装上事件处理器和「按下之后做什么」。
/// `action` 在快捷键线程上被调用，不许阻塞、不许碰界面
/// （要碰界面自己 `slint::invoke_from_event_loop`）。
pub fn init(action: impl Fn() + Send + Sync + 'static) {
    let action = Arc::new(action);
    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        let cur_id = CURRENT_ID.load(Ordering::SeqCst);
        if cur_id != 0 && event.state == HotKeyState::Pressed && event.id == cur_id {
            action();
        }
    }));
}

/// 换键。空串 = 只注销，不再注册。注册不上（多半是被别的软件占了）返回 Err。
pub fn apply(accelerator: &str) -> Result<(), Error> {
    let trimmed = accelerator.trim();
    if trimmed.is_empty() {
        MANAGER.with(|cell| {
            let mut mgr_opt = cell.borrow_mut();
            if let Some(mgr) = mgr_opt.as_mut() {
                CURRENT_HOTKEY.with(|cur_cell| {
                    if let Some(old) = cur_cell.borrow_mut().take() {
                        let _ = mgr.unregister(old); // ignore: 清空快捷键时注销已注册的旧热键
                    }
                });
            }
        });
        CURRENT_ID.store(0, Ordering::SeqCst);
        return Ok(());
    }

    let hotkey: HotKey = trimmed
        .parse()
        .map_err(|e| Error::Platform(format!("{e}")))?;

    // 检查是否与当前已注册的热键相同
    let same = CURRENT_HOTKEY.with(|cur_cell| *cur_cell.borrow() == Some(hotkey));
    if same {
        return Ok(());
    }

    MANAGER.with(|cell| {
        let mut mgr_opt = cell.borrow_mut();
        if mgr_opt.is_none() {
            *mgr_opt = Some(GlobalHotKeyManager::new().map_err(|e| {
                Error::Platform(format!("Failed to create GlobalHotKeyManager: {e}"))
            })?);
        }
        let Some(mgr) = mgr_opt.as_mut() else {
            return Err(Error::Platform("GlobalHotKeyManager not available".into()));
        };

        // 先注册新键，成功后再注销旧键（RegisterHotKey 允许同时存在两个不同的键）。
        // 这样如果新键注册失败（被占），旧键仍然保持注册状态，不破坏原有快捷键。
        mgr.register(hotkey).map_err(|e| {
            log::warn!("Hotkey: register {trimmed} failed: {e}");
            Error::Platform(e.to_string())
        })?;

        CURRENT_HOTKEY.with(|cur_cell| {
            if let Some(old) = cur_cell.borrow_mut().replace(hotkey) {
                let _ = mgr.unregister(old); // ignore: 新键注册成功后注销旧键
            }
        });
        CURRENT_ID.store(hotkey.id(), Ordering::SeqCst);
        log::info!("Hotkey: registered {trimmed}");
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn test_accelerator_roundtrip_all_categories() {
        let is_macos = std::env::consts::OS == "macos";
        let super_cmd = if is_macos { "Command" } else { "Super" };

        // 1. Letters (lowercase converted to uppercase)
        let a = accelerator("a", true, true, false, false).unwrap();
        assert_eq!(a, "Ctrl+Shift+A");
        assert!(HotKey::from_str(&a).is_ok());

        let z = accelerator("z", false, false, true, false).unwrap();
        assert_eq!(z, "Alt+Z");
        assert!(HotKey::from_str(&z).is_ok());

        // 2. Digits
        let one = accelerator("1", true, false, false, false).unwrap();
        assert_eq!(one, "Ctrl+1");
        assert!(HotKey::from_str(&one).is_ok());

        let nine = accelerator("9", false, true, false, false).unwrap();
        assert_eq!(nine, "Shift+9");
        assert!(HotKey::from_str(&nine).is_ok());

        // 3. F-keys (F1 - F24)
        let f1 = accelerator("\u{F704}", true, false, false, false).unwrap();
        assert_eq!(f1, "Ctrl+F1");
        assert!(HotKey::from_str(&f1).is_ok());

        let f12 = accelerator("\u{F70F}", false, false, true, false).unwrap();
        assert_eq!(f12, "Alt+F12");
        assert!(HotKey::from_str(&f12).is_ok());

        let f24 = accelerator("\u{F71B}", true, true, false, false).unwrap();
        assert_eq!(f24, "Ctrl+Shift+F24");
        assert!(HotKey::from_str(&f24).is_ok());

        // 4. Direction keys
        let up = accelerator("\u{F700}", true, false, false, false).unwrap();
        assert_eq!(up, "Ctrl+Up");
        assert!(HotKey::from_str(&up).is_ok());

        let down = accelerator("\u{F701}", false, true, false, false).unwrap();
        assert_eq!(down, "Shift+Down");
        assert!(HotKey::from_str(&down).is_ok());

        let left = accelerator("\u{F702}", false, false, true, false).unwrap();
        assert_eq!(left, "Alt+Left");
        assert!(HotKey::from_str(&left).is_ok());

        let right = accelerator("\u{F703}", true, true, false, false).unwrap();
        assert_eq!(right, "Ctrl+Shift+Right");
        assert!(HotKey::from_str(&right).is_ok());

        // 5. Space
        let space = accelerator(" ", true, false, false, false).unwrap();
        assert_eq!(space, "Ctrl+Space");
        assert!(HotKey::from_str(&space).is_ok());

        // 6. Special keys (Tab, Enter, Delete, Insert, Home, End, PageUp, PageDown, ScrollLock, Pause, PrintScreen)
        let tab = accelerator("\u{0009}", true, false, false, false).unwrap();
        assert_eq!(tab, "Ctrl+Tab");
        assert!(HotKey::from_str(&tab).is_ok());

        let enter = accelerator("\u{000a}", true, false, false, false).unwrap();
        assert_eq!(enter, "Ctrl+Enter");
        assert!(HotKey::from_str(&enter).is_ok());

        let del = accelerator("\u{007f}", true, false, false, false).unwrap();
        assert_eq!(del, "Ctrl+Delete");
        assert!(HotKey::from_str(&del).is_ok());

        let insert = accelerator("\u{F727}", false, true, false, false).unwrap();
        assert_eq!(insert, "Shift+Insert");
        assert!(HotKey::from_str(&insert).is_ok());

        let home = accelerator("\u{F729}", true, false, false, false).unwrap();
        assert_eq!(home, "Ctrl+Home");
        assert!(HotKey::from_str(&home).is_ok());

        let end = accelerator("\u{F72B}", true, false, false, false).unwrap();
        assert_eq!(end, "Ctrl+End");
        assert!(HotKey::from_str(&end).is_ok());

        let pgup = accelerator("\u{F72C}", true, false, false, false).unwrap();
        assert_eq!(pgup, "Ctrl+PageUp");
        assert!(HotKey::from_str(&pgup).is_ok());

        let pgdn = accelerator("\u{F72D}", true, false, false, false).unwrap();
        assert_eq!(pgdn, "Ctrl+PageDown");
        assert!(HotKey::from_str(&pgdn).is_ok());

        let scroll = accelerator("\u{F72F}", true, false, false, false).unwrap();
        assert_eq!(scroll, "Ctrl+ScrollLock");
        assert!(HotKey::from_str(&scroll).is_ok());

        let pause = accelerator("\u{F730}", true, false, false, false).unwrap();
        assert_eq!(pause, "Ctrl+Pause");
        assert!(HotKey::from_str(&pause).is_ok());

        let prt = accelerator("\u{F731}", true, false, false, false).unwrap();
        assert_eq!(prt, "Ctrl+PrintScreen");
        assert!(HotKey::from_str(&prt).is_ok());

        // Punctuation symbols
        let comma = accelerator(",", true, false, false, false).unwrap();
        assert_eq!(comma, "Ctrl+,");
        assert!(HotKey::from_str(&comma).is_ok());

        let minus = accelerator("-", true, false, false, false).unwrap();
        assert_eq!(minus, "Ctrl+-");
        assert!(HotKey::from_str(&minus).is_ok());

        let period = accelerator(".", true, false, false, false).unwrap();
        assert_eq!(period, "Ctrl+.");
        assert!(HotKey::from_str(&period).is_ok());

        let slash = accelerator("/", true, false, false, false).unwrap();
        assert_eq!(slash, "Ctrl+/");
        assert!(HotKey::from_str(&slash).is_ok());

        let bquote = accelerator("`", true, false, false, false).unwrap();
        assert_eq!(bquote, "Ctrl+`");
        assert!(HotKey::from_str(&bquote).is_ok());

        let bslash = accelerator("\\", true, false, false, false).unwrap();
        assert_eq!(bslash, "Ctrl+\\");
        assert!(HotKey::from_str(&bslash).is_ok());

        let sqbracket_l = accelerator("[", true, false, false, false).unwrap();
        assert_eq!(sqbracket_l, "Ctrl+[");
        assert!(HotKey::from_str(&sqbracket_l).is_ok());

        let sqbracket_r = accelerator("]", true, false, false, false).unwrap();
        assert_eq!(sqbracket_r, "Ctrl+]");
        assert!(HotKey::from_str(&sqbracket_r).is_ok());

        let equal = accelerator("=", true, false, false, false).unwrap();
        assert_eq!(equal, "Ctrl+=");
        assert!(HotKey::from_str(&equal).is_ok());

        let quote = accelerator("'", true, false, false, false).unwrap();
        assert_eq!(quote, "Ctrl+'");
        assert!(HotKey::from_str(&quote).is_ok());

        let semicolon = accelerator(";", true, false, false, false).unwrap();
        assert_eq!(semicolon, "Ctrl+;");
        assert!(HotKey::from_str(&semicolon).is_ok());

        // 7. Modifiers (1 to 4 combinations)
        let m1 = accelerator("A", true, false, false, false).unwrap();
        assert_eq!(m1, "Ctrl+A");
        assert!(HotKey::from_str(&m1).is_ok());

        let m2 = accelerator("A", true, true, false, false).unwrap();
        assert_eq!(m2, "Ctrl+Shift+A");
        assert!(HotKey::from_str(&m2).is_ok());

        let m3 = accelerator("A", true, true, true, false).unwrap();
        assert_eq!(m3, "Ctrl+Shift+Alt+A");
        assert!(HotKey::from_str(&m3).is_ok());

        let m4 = accelerator("A", true, true, true, true).unwrap();
        assert_eq!(m4, format!("Ctrl+Shift+{super_cmd}+Alt+A"));
        assert!(HotKey::from_str(&m4).is_ok());

        // 8. Backspace returns empty string (clear)
        assert_eq!(
            accelerator("\u{0008}", false, false, false, false),
            Some(String::new())
        );
        assert_eq!(
            accelerator("\u{0008}", true, true, false, false),
            Some(String::new())
        );

        // 9. Modifiers alone return None
        for mod_char in [
            "\u{0010}", "\u{0011}", "\u{0012}", "\u{0013}", "\u{0014}", "\u{0015}", "\u{0016}",
            "\u{0017}", "\u{0018}",
        ] {
            assert_eq!(accelerator(mod_char, true, false, false, false), None);
        }

        // 10. Escape returns None
        assert_eq!(accelerator("\u{001b}", false, false, false, false), None);
    }

    #[test]
    fn test_modifiers_only() {
        let is_macos = std::env::consts::OS == "macos";
        let super_cmd = if is_macos { "Command" } else { "Super" };

        assert_eq!(modifiers_only(false, false, false, false), "");
        assert_eq!(modifiers_only(true, false, false, false), "Ctrl");
        assert_eq!(modifiers_only(true, true, false, false), "Ctrl+Shift");
        assert_eq!(modifiers_only(true, true, true, false), "Ctrl+Shift+Alt");
        assert_eq!(
            modifiers_only(true, true, true, true),
            format!("Ctrl+Shift+{super_cmd}+Alt")
        );
    }
}
