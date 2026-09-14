//! 设置窗口（B0 是渲染验证页 + 界面语言）。打开时创建，关闭即销毁（design §1.4）。
//! 圆角验证窗口（阶段 A）跟着设置窗口一起开、一起销毁。

use std::cell::RefCell;
use std::path::Path;
use std::time::Duration;

use slint::{CloseRequestResponse, ComponentHandle};

use crate::error::Error;
use crate::logic::config::{self, LANGUAGES};
use crate::platform;
use crate::slint_ui::{RenderCheck, RoundCheck};

struct Settings {
    page: RenderCheck,
    round: RoundCheck,
}

thread_local! {
    static SETTINGS: RefCell<Option<Settings>> = const { RefCell::new(None) };
}

/// 打开设置窗口；已经开着就拉到最前面。
pub fn open() {
    open_inner(None);
}

/// 配置文件损坏被备份时：打开设置窗口，顶部提示备份文件名。
pub fn open_with_notice(backup: &Path) {
    open_inner(Some(backup));
}

fn open_inner(backup: Option<&Path>) {
    let raised = SETTINGS.with_borrow(|slot| {
        slot.as_ref()
            .map(|s| match platform::bring_to_front(s.page.window()) {
                // 平台还没实现（macOS）：至少保证窗口是显示的，和改之前一样。
                Err(Error::Unsupported) => s.page.show().map_err(Error::from),
                other => other,
            })
    });
    match raised {
        Some(Ok(())) => return,
        Some(Err(e)) => {
            log::warn!("Settings: bring to front failed: {e}");
            return;
        }
        None => {}
    }
    match create(backup) {
        Ok(settings) => {
            SETTINGS.set(Some(settings));
            log::info!("Settings: opened");
        }
        Err(e) => log::error!("Settings: create window failed: {e}"),
    }
}

fn create(backup: Option<&Path>) -> Result<Settings, Error> {
    let page = RenderCheck::new()?;
    let sample =
        "Runtime: an <font color=\"#2f9e44\">**unprecedented**</font> result，前所未有的结果 🎯";
    match slint::StyledText::from_markdown(sample) {
        Ok(styled) => page.set_runtime_sample(styled),
        Err(e) => page.set_runtime_sample(slint::StyledText::from_plain_text(&e.to_string())),
    }
    if let Some(name) = backup.and_then(Path::file_name) {
        page.set_bad_config(name.to_string_lossy().as_ref().into()); // 只用于显示
    }
    page.invoke_show_language(language_index(&config::snapshot().general.language));
    let weak = page.as_weak();
    page.on_language_selected(move |index| {
        if let Some(page) = weak.upgrade() {
            select_language(&page, index);
        }
    });
    page.window().on_close_requested(|| {
        // 不能在窗口自己的回调里销毁它：排到下一轮事件循环再 drop。
        slint::Timer::single_shot(Duration::ZERO, close);
        CloseRequestResponse::HideWindow
    });
    page.show()?;

    let round = RoundCheck::new()?;
    round.show()?;
    round_when_ready(round.as_weak(), 1);
    Ok(Settings { page, round })
}

/// 等原生窗口建好再设圆角。
/// 事件循环已经在跑时 show() 的窗口，winit 要到之后的某一轮才真正建 HWND：
/// 当场取句柄、`invoke_from_event_loop` 排队都太早（报 "underlying handle cannot be represented"）。
/// 所以用 Timer 每 10ms 试一次（实测第一次 tick 就成功），最多 50 次；窗口先被关掉就停。
fn round_when_ready(weak: slint::Weak<RoundCheck>, attempt: u32) {
    const MAX_ATTEMPTS: u32 = 50;
    slint::Timer::single_shot(Duration::from_millis(10), move || {
        let Some(round) = weak.upgrade() else { return };
        match platform::round_corners(round.window()) {
            Ok(()) => round.set_status(format!("DWM 圆角：已设置（第 {attempt} 次）").into()),
            Err(e) if attempt >= MAX_ATTEMPTS => {
                log::warn!("Settings: round corners failed after {attempt} tries: {e}");
                round.set_status(format!("圆角未生效：{e}").into());
            }
            Err(_) => round_when_ready(weak, attempt + 1),
        }
    });
}

fn close() {
    let Some(settings) = SETTINGS.take() else {
        return;
    };
    // Slint 在窗口可见期间自己持有组件，必须先 hide，drop 才真正释放。page 已经被关闭请求隐藏。
    if let Err(e) = settings.round.hide() {
        log::warn!("Settings: hide round-corner window failed: {e}");
    }
    drop(settings);
    log::info!("Settings: closed");
}

fn select_language(page: &RenderCheck, index: i32) {
    let Some(&language) = usize::try_from(index).ok().and_then(|i| LANGUAGES.get(i)) else {
        return;
    };
    let before = config::snapshot().general.language;
    match config::update(|c| c.general.language = language.to_owned()) {
        Ok(()) => {
            page.set_save_error(Default::default());
            super::apply_language(language);
        }
        Err(e) => {
            log::warn!("Settings: save language failed: {e}");
            page.set_save_error(e.to_string().into());
            page.invoke_show_language(language_index(&before));
        }
    }
}

fn language_index(language: &str) -> i32 {
    LANGUAGES.iter().position(|l| *l == language).unwrap_or(0) as i32
}
