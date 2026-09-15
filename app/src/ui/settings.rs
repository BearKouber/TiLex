//! 设置窗口（B0 是渲染验证页 + 界面语言）。打开时创建，关闭即销毁（design §1.4）。

use std::cell::RefCell;
use std::path::Path;
use std::time::Duration;

use slint::winit_030::WinitWindowAccessor;
use slint::{CloseRequestResponse, ComponentHandle};

use crate::error::Error;
use crate::logic::config::{self, LANGUAGES};
use crate::platform;
use crate::slint_ui::RenderCheck;

struct Settings {
    page: RenderCheck,
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

    let weak_drag = page.as_weak();
    page.on_drag_requested(move || {
        if let Some(page) = weak_drag.upgrade() {
            match page.window().with_winit_window(|w| w.drag_window()) {
                Some(Err(e)) => log::warn!("Settings: drag_window failed: {e}"),
                None => log::warn!("Settings: drag_window: winit window not available"),
                _ => {}
            }
        }
    });

    let weak_close = page.as_weak();
    page.on_close_clicked(move || {
        if let Some(page) = weak_close.upgrade() {
            schedule_close(&page);
        }
    });

    page.window().on_close_requested(|| {
        // Alt+F4 / 任务栏关闭：返回 HideWindow 由 Slint 隐藏，再排到下一轮 drop。
        slint::Timer::single_shot(Duration::ZERO, close);
        CloseRequestResponse::HideWindow
    });

    page.show()?;
    style_when_ready(page.as_weak(), 1);

    Ok(Settings { page })
}

/// 等原生窗口建好再应用平台无边框外框样式（Win11 圆角 + 窗口阴影）。
/// 事件循环已经在跑时 show() 的窗口，winit 要到之后的某一轮才真正建 HWND：
/// 当场取句柄、`invoke_from_event_loop` 排队都太早（报 "underlying handle cannot be represented"）。
/// 所以用 Timer 每 10ms 试一次（实测第一次 tick 就成功），最多 50 次；窗口先被关掉就停。
fn style_when_ready(weak: slint::Weak<RenderCheck>, attempt: u32) {
    const MAX_ATTEMPTS: u32 = 50;
    slint::Timer::single_shot(Duration::from_millis(10), move || {
        let Some(page) = weak.upgrade() else { return };
        match platform::style_frameless_window(page.window()) {
            Ok(()) => log::info!("Settings: styled frameless window (attempt {attempt})"),
            Err(e) if attempt >= MAX_ATTEMPTS => {
                log::warn!("Settings: style frameless window failed after {attempt} tries: {e}");
            }
            Err(_) => style_when_ready(weak, attempt + 1),
        }
    });
}

/// 关闭 = 销毁（design §1.4）。不能在窗口自己的回调里 drop：先隐藏（Slint 在窗口可见期间自己持有组件，
/// 不隐藏 drop 不掉），排到下一轮事件循环再 drop。
fn schedule_close(page: &RenderCheck) {
    if let Err(e) = page.hide() {
        log::warn!("Settings: hide window failed: {e}");
    }
    slint::Timer::single_shot(Duration::ZERO, close);
}

fn close() {
    let Some(settings) = SETTINGS.take() else {
        return;
    };
    drop(settings); // 已经隐藏过（关闭请求返回 HideWindow，或 schedule_close）

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
