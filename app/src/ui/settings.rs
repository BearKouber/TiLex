//! 设置窗口。打开时创建，关闭即销毁（design §1.4）。

use std::cell::RefCell;
use std::path::Path;
use std::time::Duration;

use slint::winit_030::WinitWindowAccessor;
use slint::{CloseRequestResponse, ComponentHandle};

use crate::error::Error;
use crate::logic::config;
use crate::platform;
use crate::slint_ui::SettingsWindow;

struct Settings {
    page: SettingsWindow,
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
    let page = SettingsWindow::new()?;

    let cfg = config::snapshot();
    page.set_theme_mode(cfg.general.theme.as_str().into());
    page.set_language(cfg.general.language.as_str().into());
    let scheme = super::resolve_color_scheme(page.window());
    page.set_color_scheme(scheme);

    let weak_theme = page.as_weak();
    page.on_cycle_theme(move || {
        if let Some(page) = weak_theme.upgrade() {
            cycle_theme(&page);
        }
    });

    let weak_lang = page.as_weak();
    page.on_toggle_language(move || {
        if let Some(page) = weak_lang.upgrade() {
            toggle_language(&page);
        }
    });

    let weak_drag = page.as_weak();
    page.on_drag(move || {
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

    page.set_version(env!("CARGO_PKG_VERSION").into());
    match platform::autostart_enabled() {
        Ok(enabled) => {
            page.set_autostart_supported(true);
            page.set_autostart_enabled(enabled);
        }
        Err(Error::Unsupported) => {
            page.set_autostart_supported(false);
            page.set_autostart_enabled(false);
        }
        Err(e) => {
            log::warn!("Settings: check autostart failed: {e}");
            page.set_autostart_supported(true);
            page.set_autostart_enabled(false);
        }
    }
    update_credits(&page);

    let weak_url = page.as_weak();
    page.on_open_url(move |url| {
        if let Some(page) = weak_url.upgrade() {
            open_url_or_toast(&page, &url);
        }
    });

    let weak_gh = page.as_weak();
    page.on_open_github(move || {
        if let Some(page) = weak_gh.upgrade() {
            open_url_or_toast(&page, REPO);
        }
    });

    let weak_fb = page.as_weak();
    page.on_open_feedback(move || {
        if let Some(page) = weak_fb.upgrade() {
            open_url_or_toast(&page, &format!("{REPO}/issues"));
        }
    });

    let weak_rel = page.as_weak();
    page.on_open_releases(move || {
        if let Some(page) = weak_rel.upgrade() {
            open_url_or_toast(&page, &format!("{REPO}/releases"));
        }
    });

    let weak_auto = page.as_weak();
    page.on_autostart_toggled(move |on| {
        if let Some(page) = weak_auto.upgrade() {
            handle_autostart_toggle(&page, on);
        }
    });

    let weak_upd = page.as_weak();
    page.on_check_update(move || {
        if let Some(page) = weak_upd.upgrade() {
            check_update(&page);
        }
    });

    let weak_log = page.as_weak();
    page.on_view_log(move || {
        if let Some(page) = weak_log.upgrade() {
            view_log(&page);
        }
    });

    let weak_cfg = page.as_weak();
    page.on_view_config(move || {
        if let Some(page) = weak_cfg.upgrade() {
            view_config(&page);
        }
    });

    if let Some(name) = backup.and_then(Path::file_name) {
        page.invoke_show_bad_config(name.to_string_lossy().as_ref().into());
    }

    Ok(Settings { page })
}

/// 等原生窗口建好再应用平台无边框外框样式（Win11 圆角 + 窗口阴影）。
/// 事件循环已经在跑时 show() 的窗口，winit 要到之后的某一轮才真正建 HWND：
/// 当场取句柄、`invoke_from_event_loop` 排队都太早（报 "underlying handle cannot be represented"）。
/// 所以用 Timer 每 10ms 试一次（实测第一次 tick 就成功），最多 50 次；窗口先被关掉就停。
fn style_when_ready(weak: slint::Weak<SettingsWindow>, attempt: u32) {
    const MAX_ATTEMPTS: u32 = 50;
    slint::Timer::single_shot(Duration::from_millis(10), move || {
        let Some(page) = weak.upgrade() else { return };
        match platform::style_frameless_window(page.window()) {
            Ok(()) => {
                log::info!("Settings: styled frameless window (attempt {attempt})");
                // 原生窗口这时才有：show() 之前问不到系统主题，"跟随系统"要在这里再算一次
                page.set_color_scheme(super::resolve_color_scheme(page.window()));
            }
            Err(e) if attempt >= MAX_ATTEMPTS => {
                log::warn!("Settings: style frameless window failed after {attempt} tries: {e}");
            }
            Err(_) => style_when_ready(weak, attempt + 1),
        }
    });
}

/// 关闭 = 销毁（design §1.4）。不能在窗口自己的回调里 drop：先隐藏（Slint 在窗口可见期间自己持有组件，
/// 不隐藏 drop 不掉），排到下一轮事件循环再 drop。
fn schedule_close(page: &SettingsWindow) {
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

fn cycle_theme(page: &SettingsWindow) {
    let current = config::snapshot().general.theme;
    let next = match current.as_str() {
        "light" => "dark",
        "dark" => "system",
        _ => "light",
    };
    match config::update(|c| c.general.theme = next.to_owned()) {
        Ok(()) => {
            page.set_theme_mode(next.into());
            let scheme = super::resolve_color_scheme(page.window());
            page.set_color_scheme(scheme);
        }
        Err(e) => {
            log::warn!("Settings: save theme failed: {e}");
            page.invoke_show_save_error(e.to_string().into());
        }
    }
}

fn toggle_language(page: &SettingsWindow) {
    let current = config::snapshot().general.language;
    let next = if current == "zh_CN" { "en" } else { "zh_CN" };
    match config::update(|c| c.general.language = next.to_owned()) {
        Ok(()) => {
            page.set_language(next.into());
            super::apply_language(next);
            update_credits(page);
        }
        Err(e) => {
            log::warn!("Settings: save language failed: {e}");
            page.invoke_show_save_error(e.to_string().into());
        }
    }
}

const REPO: &str = "https://github.com/BearKouber/TiLex";

fn update_credits(page: &SettingsWindow) {
    let raw = page.get_credits_raw();
    let styled = slint::StyledText::from_markdown(&raw)
        .unwrap_or_else(|_| slint::StyledText::from_plain_text(&raw));
    page.set_credits(styled);
}

fn open_url_or_toast(page: &SettingsWindow, url: &str) {
    if let Err(e) = platform::open_url(url) {
        log::warn!("Settings: open url {url:?} failed: {e}");
        page.invoke_show_open_url_error(e.to_string().into());
    }
}

fn view_log(page: &SettingsWindow) {
    if let Err(e) = super::open_log_dir() {
        log::warn!("Settings: open log folder failed: {e}");
        page.invoke_show_open_path_error(e.to_string().into());
    }
}

fn view_config(page: &SettingsWindow) {
    match platform::data_dir() {
        Ok(dir) => {
            if let Err(e) = platform::open_path(&dir) {
                log::warn!("Settings: open config folder failed: {e}");
                page.invoke_show_open_path_error(e.to_string().into());
            }
        }
        Err(e) => {
            log::warn!("Settings: get data dir failed: {e}");
            page.invoke_show_open_path_error(e.to_string().into());
        }
    }
}

fn handle_autostart_toggle(page: &SettingsWindow, on: bool) {
    match platform::set_autostart(on) {
        Ok(()) => {
            log::info!("Settings: autostart set to {on}");
            page.set_autostart_enabled(on);
        }
        Err(e) => {
            log::warn!("Settings: set autostart to {on} failed: {e}");
            page.invoke_show_autostart_error(e.to_string().into());
            page.set_autostart_enabled(!on);
        }
    }
}

fn check_update(page: &SettingsWindow) {
    if page.get_checking_update() {
        return;
    }
    page.set_checking_update(true);
    let weak = page.as_weak();
    if let Err(e) = std::thread::Builder::new()
        .name("check-update".into())
        .spawn(move || {
            let res = crate::logic::update::check();
            // ignore: window might be closed during async check
            let _ = slint::invoke_from_event_loop(move || {
                let Some(page) = weak.upgrade() else { return };
                page.set_checking_update(false);
                match res {
                    Ok(crate::logic::update::Release::Newer(v)) => {
                        page.invoke_show_update_newer(v.into());
                    }
                    Ok(crate::logic::update::Release::Latest) => {
                        page.invoke_show_update_latest();
                    }
                    Ok(crate::logic::update::Release::None) => {
                        page.invoke_show_update_none();
                    }
                    Err(e) => {
                        log::warn!("Settings: check update failed: {e}");
                        page.invoke_show_update_failed(e.to_string().into());
                    }
                }
            });
        })
    {
        log::error!("Settings: spawn check-update thread failed: {e}");
        page.set_checking_update(false);
        page.invoke_show_update_failed(e.to_string().into());
    }
}
