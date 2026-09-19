//! 增强选中识别（`selection.force_copy`，默认关）。
//! 有的程序自己画文字，Accessibility (AX) API 读不到选区。
//! 开关打开、且焦点控件无 `AXSelectedText` 属性时，模拟一次 Cmd+C 读出文字，再把原来的剪贴板写回去。
//! 契约见 platform-windows.md §4 及 04-selection-fallback.md。
//!
//! 文字当 AX/UIA 答案交给状态机（`complete_uia`），不当剪贴板候选：
//! 恢复会改剪贴板 changeCount，剪贴板那条路复核序号时会把浮标立刻收回。
//! 只在取词 worker 上跑。日志只记大小和耗时，不记内容。

use std::time::{Duration, Instant};

use objc2_core_graphics::{
    CGEvent, CGEventFlags, CGEventSource, CGEventSourceStateID, CGEventTapLocation,
};

use super::pasteboard::{self, Snapshot};

/// 模拟 Cmd+C 有害、也拿不到文字的程序。
/// 匹配规则同黑名单（应用名称或 bundle id 末段，不区分大小写）；只影响模拟复制，不影响常规 AX 选区与浮标。
///
/// macOS 与 Windows 的关键差异：
/// 1. 终端（Terminal / iTerm2 / Alacritty 等）：
///    在 Windows 上 Ctrl+C 是 SIGINT 中断信号；而在 macOS 终端中，中断为 Control+C，
///    复制快捷键为标准的 Command+C。Cmd+C 在 macOS 终端中仅执行复制，若无选区则静默无害，因此终端无需列入。
/// 2. 访达（Finder）：
///    在桌面或访达窗口中按 Cmd+C 复制的是选中的文件或文件夹（生成文件 URL/Promise），
///    绝非文字选区，且会在第三方剪贴板历史工具中产生文件项；双击桌面空白或划选文件易误入此路径，必须排除。
pub const NO_FORCE_COPY: &str = "Finder";

/// 虚拟键码：macOS ANSI 'C' 虚拟键码为 8（HIToolbox / Events.h）。
const KEYCODE_C: u16 = 8;
const STEP: Duration = Duration::from_millis(10);
/// 小于 CLIP_GRACE_MS（600ms），目标程序晚写的复制还能走剪贴板被动监听那条路。
const COPY_WAIT: Duration = Duration::from_millis(500);
/// 等目标程序把其他格式写完再打开并读取剪贴板。
const SETTLE: Duration = Duration::from_millis(30);

pub struct Inputs {
    pub enabled: bool,
    pub has_selected_text_attr: bool,
    pub clipboard_changed: bool,
    pub modifiers_down: bool,
    /// 前台进程在 NO_FORCE_COPY 里
    pub excluded_app: bool,
}

pub fn eligible(i: &Inputs) -> bool {
    // 有 AXSelectedText 属性但 AX 读出为空 = 真的没选中。剪贴板变了 = 程序选中即复制，被动监听那边会处理。
    i.enabled
        && !i.has_selected_text_attr
        && !i.clipboard_changed
        && !i.modifiers_down
        && !i.excluded_app
}

/// 检查修饰键（Command / Shift / Control / Option）是否被按下。
pub fn modifiers_down() -> bool {
    let flags = CGEventSource::flags_state(CGEventSourceStateID::CombinedSessionState);
    flags.intersects(
        CGEventFlags::MaskCommand
            | CGEventFlags::MaskShift
            | CGEventFlags::MaskControl
            | CGEventFlags::MaskAlternate,
    )
}

/// 合成一次 Cmd+C 按键事件（先按 C↓ 带 Cmd flag，再发 C↑ 带 Cmd flag）。
///
/// 生产环境仅在满足 eligible 条件且当前手势有效时执行。
/// 严禁编写任何在测试中直接或间接调用此函数的测试用例，以免向宿主系统发送击键。
pub fn send_cmd_c() -> bool {
    let Some(source) = CGEventSource::new(CGEventSourceStateID::HIDSystemState) else {
        return false;
    };
    let Some(down) = CGEvent::new_keyboard_event(Some(&source), KEYCODE_C, true) else {
        return false;
    };
    let Some(up) = CGEvent::new_keyboard_event(Some(&source), KEYCODE_C, false) else {
        return false;
    };
    CGEvent::set_flags(Some(&down), CGEventFlags::MaskCommand);
    CGEvent::set_flags(Some(&up), CGEventFlags::MaskCommand);
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&down));
    CGEvent::post(CGEventTapLocation::HIDEventTap, Some(&up));
    true
}

/// 备份 → Cmd+C → 等序号变化 → 读文字 → 写回备份。
///
/// `still_current` 在按键发出前再查一次：快照较大时备份耗时可能导致手势失效。
pub fn copy_selection(still_current: impl Fn() -> bool) -> String {
    let started = Instant::now();
    let Some(snapshot) = Snapshot::take() else {
        return String::new();
    };
    let before = pasteboard::current_change_count();
    if !still_current() {
        return String::new();
    }
    if !send_cmd_c() {
        log::debug!("PopButton: force copy input was blocked");
        return String::new();
    }
    let deadline = Instant::now() + COPY_WAIT;
    loop {
        std::thread::sleep(STEP);
        if pasteboard::current_change_count() != before {
            break;
        }
        if Instant::now() >= deadline {
            log::debug!("PopButton: force copy saw no pasteboard change");
            return String::new();
        }
    }
    std::thread::sleep(SETTLE);
    let Some(text) = pasteboard::read_text() else {
        return String::new();
    };
    let copied = pasteboard::current_change_count();
    let restored = snapshot.restore(copied);
    log::info!(
        "PopButton: force copy read {} chars, {} items backed up, restored {}, {} ms",
        text.chars().count(),
        snapshot.item_count(),
        restored,
        started.elapsed().as_millis(),
    );
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::macos::selection::matches_blacklist;

    fn inputs() -> Inputs {
        Inputs {
            enabled: true,
            has_selected_text_attr: false,
            clipboard_changed: false,
            modifiers_down: false,
            excluded_app: false,
        }
    }

    #[test]
    fn test_eligible_all_conditions_met() {
        assert!(eligible(&inputs()));
    }

    #[test]
    fn test_eligible_individually_blocked() {
        // 每个条件单独拦截验证
        assert!(!eligible(&Inputs {
            enabled: false,
            ..inputs()
        }));
        assert!(!eligible(&Inputs {
            has_selected_text_attr: true,
            ..inputs()
        }));
        assert!(!eligible(&Inputs {
            clipboard_changed: true,
            ..inputs()
        }));
        assert!(!eligible(&Inputs {
            modifiers_down: true,
            ..inputs()
        }));
        assert!(!eligible(&Inputs {
            excluded_app: true,
            ..inputs()
        }));
    }

    #[test]
    fn test_no_force_copy_matching() {
        assert!(matches_blacklist(Some("Finder"), None, NO_FORCE_COPY));
        assert!(matches_blacklist(Some("finder"), None, NO_FORCE_COPY));
        assert!(matches_blacklist(
            None,
            Some("com.apple.finder"),
            NO_FORCE_COPY
        ));
        assert!(matches_blacklist(
            Some("访达"),
            Some("com.apple.finder"),
            NO_FORCE_COPY
        ));

        // 终端与普通应用不进 NO_FORCE_COPY
        assert!(!matches_blacklist(Some("Terminal"), None, NO_FORCE_COPY));
        assert!(!matches_blacklist(Some("iTerm2"), None, NO_FORCE_COPY));
        assert!(!matches_blacklist(Some("Alacritty"), None, NO_FORCE_COPY));
        assert!(!matches_blacklist(Some("kitty"), None, NO_FORCE_COPY));
        assert!(!matches_blacklist(Some("Safari"), None, NO_FORCE_COPY));
    }
}
