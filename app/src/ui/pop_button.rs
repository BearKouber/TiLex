//! 划词浮标窗口。启动时建好（D12），在屏幕外 `show()` 一次，拿到原生窗口后交给平台层
//! （`platform::attach_selection_button`），之后显示、隐藏、位置都由平台层管，这里再也不调它的 `show()`/`hide()`。

use std::cell::RefCell;
use std::time::Duration;

use slint::{CloseRequestResponse, ComponentHandle, PhysicalPosition};

use crate::error::Error;
use crate::logic::config;
use crate::platform::geometry::Side;
use crate::platform::{self, EngagedSelection, SelectionSettings};
use crate::slint_ui::PopButton;

thread_local! {
    static BUTTON: RefCell<Option<slint::Weak<PopButton>>> = const { RefCell::new(None) };
}

/// 启动划词监听并建浮标。返回 `None`：这个平台还不支持划词（macOS 在 B6），或监听没起来（已记日志）。
/// 调用方持有返回值到退出。
pub fn create() -> Result<Option<PopButton>, Error> {
    match platform::start_selection(settings, accept, engaged, before_show) {
        Ok(()) => {}
        Err(Error::Unsupported) => {
            log::info!("PopButton: selection is not supported on this platform yet");
            return Ok(None);
        }
        Err(e) => {
            log::error!("PopButton: start selection failed: {e}");
            return Ok(None);
        }
    }
    super::CREATING_INACTIVE.set(true);
    let button = PopButton::new();
    super::CREATING_INACTIVE.set(false);
    let button = button?;
    BUTTON.with(|b| *b.borrow_mut() = Some(button.as_weak()));
    button.on_hovered(|| engage_if(true));
    button.on_clicked(|| engage_if(false));
    // Slint 的 hide() 会让 winit 重写窗口样式（浮标就会抢焦点），所以永远不让它自己隐藏。
    button
        .window()
        .on_close_requested(|| CloseRequestResponse::KeepWindowShown);
    // 第一次 show 在屏幕外：原生窗口建出来、交给平台层之前那一下不会闪到屏幕上。
    button
        .window()
        .set_position(PhysicalPosition::new(-32000, -32000));
    button.show()?;
    attach_when_ready(button.as_weak(), 1);
    Ok(Some(button))
}

/// 原生窗口要到事件循环之后的某一轮才有（architecture.md §6 第 5 条），每 10ms 试一次，最多 50 次。
fn attach_when_ready(weak: slint::Weak<PopButton>, attempt: u32) {
    const MAX_ATTEMPTS: u32 = 50;
    slint::Timer::single_shot(Duration::from_millis(10), move || {
        let Some(button) = weak.upgrade() else { return };
        match platform::attach_selection_button(button.window()) {
            Ok(()) => {}
            Err(e) if attempt >= MAX_ATTEMPTS => {
                log::error!(
                    "PopButton: attach window failed after {attempt} tries, no button: {e}"
                );
            }
            Err(_) => attach_when_ready(weak, attempt + 1),
        }
    });
}

/// 悬停和点击各自只在对应的触发方式下生效。不认识的触发方式按默认的悬停。
fn engage_if(hovered: bool) {
    let hover_trigger = config::snapshot().selection.trigger != "click";
    if hovered == hover_trigger {
        platform::engage_selection();
    }
}

/// 取词 worker 每次手势现取（改了 config.json 重启后生效；B2 设置页改了立即生效）。
fn settings() -> SelectionSettings {
    let s = config::snapshot().selection;
    SelectionSettings {
        enabled: s.enabled,
        blacklist: s.blacklist,
        force_copy: s.force_copy,
        corner: corner(&s.button_pos).unwrap_or((Side::Before, Side::After)),
        gap: s.button_distance.clamp(0, 50) as i32,
    }
}

pub(crate) fn corner(pos: &str) -> Option<(Side, Side)> {
    match pos {
        "BottomRight" => Some((Side::After, Side::After)),
        "BottomLeft" => Some((Side::Before, Side::After)),
        "TopRight" => Some((Side::After, Side::Before)),
        "TopLeft" => Some((Side::Before, Side::Before)),
        // 不认识的返回 None，由调用方提供默认值
        _ => None,
    }
}

/// 排除母语（`selection.exclude_native`）的挂钩点，在取词 worker 线程上调用；返回 `false` 就不出浮标。
fn accept(text: &str) -> bool {
    let cfg = config::snapshot();
    if !cfg.selection.exclude_native {
        return true;
    }
    !crate::logic::lang_detect::is_native_language(text, &cfg.translate.target)
}

/// 用户悬停/点击了浮标（取词 worker 线程）。把文字交给结果浮窗。
fn engaged(selection: EngagedSelection) {
    log::info!(
        "PopButton: engaged, {} chars, button at ({}, {})",
        selection.text.chars().count(),
        selection.x,
        selection.y
    );
    let text = selection.text;
    let (x, y) = (selection.x, selection.y);
    if let Err(e) = slint::invoke_from_event_loop(move || {
        super::pop_result::show(&text, x, y);
    }) {
        log::warn!("PopButton: invoke pop_result::show failed: {e}");
    }
}

/// 浮标每次显示之前调用（在取词 worker 线程上）。
/// 切回 UI 线程翻转浮标的 `repaint-tick`，促使 Slint 软件渲染整窗重画，
/// 避免 DWM cloak 期间系统丢失画面后变成透明空框（platform-windows.md §1 第 4 条）。
fn before_show() {
    if let Err(e) = slint::invoke_from_event_loop(|| {
        BUTTON.with(|b| {
            if let Some(button) = b.borrow().as_ref().and_then(|w| w.upgrade()) {
                button.set_repaint_tick(!button.get_repaint_tick());
            }
        });
    }) {
        log::warn!("PopButton: invoke before_show repaint failed: {e}");
    }
}
