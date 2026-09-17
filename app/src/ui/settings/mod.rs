//! 设置窗口。打开时创建，关闭即销毁（design §1.4）。

use std::cell::RefCell;
use std::path::Path;
use std::time::Duration;

use slint::winit_030::WinitWindowAccessor;
use slint::{CloseRequestResponse, ComponentHandle};

use crate::error::Error;
use crate::logic::config;
use crate::platform;
use crate::slint_ui::{SettingsWindow, TranslateSettings};

mod services;

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

    let ts = page.global::<TranslateSettings>();
    debug_assert_eq!(
        ts.get_language_option_count(),
        SOURCE_LANGUAGES.len() as i32
    );
    debug_assert_eq!(
        ts.get_target_language_option_count(),
        TARGET_LANGUAGES.len() as i32
    );

    let source_idx = find_index(&SOURCE_LANGUAGES, &cfg.translate.source);
    let target_idx = find_index(&TARGET_LANGUAGES, &cfg.translate.target);
    let exclude_native_idx = if cfg.selection.exclude_native { 0 } else { 1 };
    let detect_idx = find_index(&DETECT_ENGINES, &cfg.translate.detect_engine);
    let pop_btn_idx = pop_button_to_index(&cfg.selection);
    let pop_btn_pos_idx = find_index(&POP_BUTTON_POS, &cfg.selection.button_pos);
    let pop_res_pos_idx = find_index(&POP_RESULT_POS, &cfg.selection.result_pos);
    let screenshot_pos_idx = find_index(&SCREENSHOT_POS, &cfg.screenshot.result_pos);
    let force_copy_idx = if cfg.selection.force_copy { 0 } else { 1 };
    let distance = cfg.selection.button_distance.clamp(0, 20) as i32;

    ts.set_source_lang_index(source_idx);
    ts.set_target_lang_index(target_idx);
    ts.set_exclude_native_index(exclude_native_idx);
    ts.set_detect_engine_index(detect_idx);
    ts.set_pop_button_index(pop_btn_idx);
    ts.set_pop_button_pos_index(pop_btn_pos_idx);
    ts.set_pop_result_pos_index(pop_res_pos_idx);
    ts.set_screenshot_pos_index(screenshot_pos_idx);
    ts.set_force_copy_index(force_copy_idx);
    ts.set_button_distance(distance);
    ts.set_blacklist(cfg.selection.blacklist.into());
    ts.set_hotkey(cfg.screenshot.hotkey.as_str().into());

    let weak_source_lang = page.as_weak();
    ts.on_source_lang_changed(move |idx| {
        if let Some(page) = weak_source_lang.upgrade() {
            handle_source_lang_change(&page, idx);
        }
    });

    let weak_target_lang = page.as_weak();
    ts.on_target_lang_changed(move |idx| {
        if let Some(page) = weak_target_lang.upgrade() {
            handle_target_lang_change(&page, idx);
        }
    });

    let weak_exclude_native = page.as_weak();
    ts.on_exclude_native_changed(move |idx| {
        if let Some(page) = weak_exclude_native.upgrade() {
            handle_exclude_native_change(&page, idx);
        }
    });

    let weak_detect_engine = page.as_weak();
    ts.on_detect_engine_changed(move |idx| {
        if let Some(page) = weak_detect_engine.upgrade() {
            handle_detect_engine_change(&page, idx);
        }
    });

    let weak_pop_button = page.as_weak();
    ts.on_pop_button_changed(move |idx| {
        if let Some(page) = weak_pop_button.upgrade() {
            handle_pop_button_change(&page, idx);
        }
    });

    let weak_pop_btn_pos = page.as_weak();
    ts.on_pop_button_pos_changed(move |idx| {
        if let Some(page) = weak_pop_btn_pos.upgrade() {
            handle_pop_button_pos_change(&page, idx);
        }
    });

    let weak_pop_res_pos = page.as_weak();
    ts.on_pop_result_pos_changed(move |idx| {
        if let Some(page) = weak_pop_res_pos.upgrade() {
            handle_pop_result_pos_change(&page, idx);
        }
    });

    let weak_screenshot_pos = page.as_weak();
    ts.on_screenshot_pos_changed(move |idx| {
        if let Some(page) = weak_screenshot_pos.upgrade() {
            handle_screenshot_pos_change(&page, idx);
        }
    });

    let weak_force_copy = page.as_weak();
    ts.on_force_copy_changed(move |idx| {
        if let Some(page) = weak_force_copy.upgrade() {
            handle_force_copy_change(&page, idx);
        }
    });

    let weak_button_distance = page.as_weak();
    ts.on_button_distance_changed(move |val| {
        if let Some(page) = weak_button_distance.upgrade() {
            handle_button_distance_change(&page, val);
        }
    });

    let weak_blacklist = page.as_weak();
    ts.on_blacklist_changed(move |text| {
        if let Some(page) = weak_blacklist.upgrade() {
            handle_blacklist_change(&page, text);
        }
    });

    let weak_hk_focus = page.as_weak();
    ts.on_hotkey_focus_changed(move |focused| {
        if let Some(page) = weak_hk_focus.upgrade() {
            handle_hotkey_focus(&page, focused);
        }
    });

    let weak_hk_key = page.as_weak();
    ts.on_hotkey_key(move |text, ctrl, shift, alt, meta| {
        if let Some(page) = weak_hk_key.upgrade() {
            handle_hotkey_key(&page, text.as_str(), ctrl, shift, alt, meta)
        } else {
            false
        }
    });

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

    let weak_restored = page.as_weak();
    page.on_restored(move || {
        if let Some(page) = weak_restored.upgrade() {
            center_on_screen(page.window());
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

    services::bind(&page);

    Ok(Settings { page })
}

/// 把窗口挪到它所在显示器工作区的正中。无边框窗口没有系统的位置记忆：
/// 首次显示和从任务栏还原都会落在左上角（B2 F7 手测），两处都调这里。
fn center_on_screen(window: &slint::Window) {
    let pos = window.position();
    let Some((area, _scale)) = platform::monitor_at(pos.x, pos.y) else {
        log::warn!(
            "Settings: no monitor at {},{}; leaving window where it is",
            pos.x,
            pos.y
        );
        return;
    };
    let size = window.size();
    window.set_position(slint::PhysicalPosition::new(
        area.l + (area.r - area.l - size.width as i32) / 2,
        area.t + (area.b - area.t - size.height as i32) / 2,
    ));
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
                center_on_screen(page.window());
                // 也是在这里才抢得到前台。从托盘点开时我们不是前台进程，Windows 不让直接抢焦点，
                // 光 show() 只会在任务栏闪一个按钮，还得用户再点一下（用户实测）。
                // 窗口已经开着的那条路在 `open_inner` 里抢过了，这里管的是第一次建窗口。
                match platform::bring_to_front(page.window()) {
                    Ok(()) | Err(Error::Unsupported) => {}
                    Err(e) => log::warn!("Settings: bring to front after create failed: {e}"),
                }
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

    // 关窗时如果还在录制快捷键，当前键已经被注销了，而失焦回调不保证还会触发
    // （Alt+F4、任务栏关闭都是直接销毁）。所有关闭路径都汇到这里，在这儿把配置里的键装回去。
    // 没在录制时这一步是空操作：`apply` 发现要装的就是当前这个键会直接返回。
    let cur = config::snapshot().screenshot.hotkey;
    if !cur.is_empty()
        && let Err(e) = crate::logic::hotkey::apply(&cur)
    {
        log::warn!("Settings: restore hotkey on close failed: {e}");
    }

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

// 对应 ui/settings/window.slint 的 language-options
const SOURCE_LANGUAGES: [&str; 31] = [
    "auto", "zh_cn", "zh_tw", "mn_mo", "en", "ja", "ko", "fr", "es", "ru", "de", "it", "tr",
    "pt_pt", "pt_br", "vi", "id", "th", "ms", "ar", "hi", "km", "mn_cy", "nb_no", "nn_no", "fa",
    "sv", "pl", "nl", "uk", "he",
];

// 对应 ui/settings/window.slint 的 target-language-options
const TARGET_LANGUAGES: [&str; 30] = [
    "zh_cn", "zh_tw", "mn_mo", "en", "ja", "ko", "fr", "es", "ru", "de", "it", "tr", "pt_pt",
    "pt_br", "vi", "id", "th", "ms", "ar", "hi", "km", "mn_cy", "nb_no", "nn_no", "fa", "sv", "pl",
    "nl", "uk", "he",
];

// 对应 ui/settings/translate.slint 的 detect-engine-options
const DETECT_ENGINES: [&str; 4] = ["local", "niutrans", "baidu", "google"];

// 对应 ui/settings/translate.slint 的 pop-button-pos-options
const POP_BUTTON_POS: [&str; 4] = ["BottomLeft", "BottomRight", "TopRight", "TopLeft"];

// 对应 ui/settings/translate.slint 的 pop-result-pos-options
const POP_RESULT_POS: [&str; 4] = ["BottomRight", "BottomLeft", "TopRight", "TopLeft"];

// 对应 ui/settings/translate.slint 的 screenshot-pos-options
const SCREENSHOT_POS: [&str; 7] = [
    "box_bottom_left",
    "box_right_top",
    "box_bottom_right",
    "cursor_bottom_right",
    "cursor_bottom_left",
    "cursor_top_right",
    "cursor_top_left",
];

fn find_index(list: &[&str], val: &str) -> i32 {
    list.iter().position(|&x| x == val).unwrap_or(0) as i32
}

fn pop_button_to_index(sel: &crate::logic::config::Selection) -> i32 {
    if !sel.enabled {
        0
    } else if sel.trigger == "click" {
        2
    } else {
        1
    }
}

/// 写配置；失败时 toast 一次并返回 false，由调用方把控件恢复成已保存的值。
fn save(
    page: &SettingsWindow,
    what: &str,
    change: impl FnOnce(&mut crate::logic::config::Config),
) -> bool {
    match config::update(change) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("Settings: save {what} failed: {e}");
            page.invoke_show_save_error(e.to_string().into());
            false
        }
    }
}

fn handle_source_lang_change(page: &SettingsWindow, idx: i32) {
    let old_cfg = config::snapshot();
    let old_idx = find_index(&SOURCE_LANGUAGES, &old_cfg.translate.source);
    if let Some(&code) = SOURCE_LANGUAGES.get(idx as usize)
        && !save(page, "source language", |c| {
            c.translate.source = code.to_owned()
        })
    {
        page.global::<TranslateSettings>()
            .set_source_lang_index(old_idx);
    }
}

fn handle_target_lang_change(page: &SettingsWindow, idx: i32) {
    let old_cfg = config::snapshot();
    let old_idx = find_index(&TARGET_LANGUAGES, &old_cfg.translate.target);
    if let Some(&code) = TARGET_LANGUAGES.get(idx as usize)
        && !save(page, "target language", |c| {
            c.translate.target = code.to_owned()
        })
    {
        page.global::<TranslateSettings>()
            .set_target_lang_index(old_idx);
    }
}

fn handle_exclude_native_change(page: &SettingsWindow, idx: i32) {
    let old_val = config::snapshot().selection.exclude_native;
    let old_idx = if old_val { 0 } else { 1 };
    let new_val = idx == 0;
    if !save(page, "exclude_native", |c| {
        c.selection.exclude_native = new_val
    }) {
        page.global::<TranslateSettings>()
            .set_exclude_native_index(old_idx);
    }
}

fn handle_detect_engine_change(page: &SettingsWindow, idx: i32) {
    let old_cfg = config::snapshot();
    let old_idx = find_index(&DETECT_ENGINES, &old_cfg.translate.detect_engine);
    if let Some(&engine) = DETECT_ENGINES.get(idx as usize)
        && !save(page, "detect_engine", |c| {
            c.translate.detect_engine = engine.to_owned()
        })
    {
        page.global::<TranslateSettings>()
            .set_detect_engine_index(old_idx);
    }
}

fn handle_pop_button_change(page: &SettingsWindow, idx: i32) {
    let old_cfg = config::snapshot();
    let old_idx = pop_button_to_index(&old_cfg.selection);
    if !save(page, "pop_button", |c| {
        if idx == 0 {
            c.selection.enabled = false;
        } else if idx == 1 {
            c.selection.enabled = true;
            c.selection.trigger = "hover".into();
        } else if idx == 2 {
            c.selection.enabled = true;
            c.selection.trigger = "click".into();
        }
    }) {
        page.global::<TranslateSettings>()
            .set_pop_button_index(old_idx);
    }
}

fn handle_pop_button_pos_change(page: &SettingsWindow, idx: i32) {
    let old_cfg = config::snapshot();
    let old_idx = find_index(&POP_BUTTON_POS, &old_cfg.selection.button_pos);
    if let Some(&pos) = POP_BUTTON_POS.get(idx as usize)
        && !save(page, "button_pos", |c| {
            c.selection.button_pos = pos.to_owned()
        })
    {
        page.global::<TranslateSettings>()
            .set_pop_button_pos_index(old_idx);
    }
}

fn handle_pop_result_pos_change(page: &SettingsWindow, idx: i32) {
    let old_cfg = config::snapshot();
    let old_idx = find_index(&POP_RESULT_POS, &old_cfg.selection.result_pos);
    if let Some(&pos) = POP_RESULT_POS.get(idx as usize)
        && !save(page, "result_pos", |c| {
            c.selection.result_pos = pos.to_owned()
        })
    {
        page.global::<TranslateSettings>()
            .set_pop_result_pos_index(old_idx);
    }
}

fn handle_screenshot_pos_change(page: &SettingsWindow, idx: i32) {
    let old_cfg = config::snapshot();
    let old_idx = find_index(&SCREENSHOT_POS, &old_cfg.screenshot.result_pos);
    if let Some(&pos) = SCREENSHOT_POS.get(idx as usize)
        && !save(page, "screenshot result_pos", |c| {
            c.screenshot.result_pos = pos.to_owned()
        })
    {
        page.global::<TranslateSettings>()
            .set_screenshot_pos_index(old_idx);
    }
}

fn handle_force_copy_change(page: &SettingsWindow, idx: i32) {
    let old_val = config::snapshot().selection.force_copy;
    let old_idx = if old_val { 0 } else { 1 };
    let new_val = idx == 0;
    if !save(page, "force_copy", |c| c.selection.force_copy = new_val) {
        page.global::<TranslateSettings>()
            .set_force_copy_index(old_idx);
    }
}

fn handle_button_distance_change(page: &SettingsWindow, val: i32) {
    let old_val = config::snapshot().selection.button_distance.clamp(0, 20) as i32;
    if !save(page, "button_distance", |c| {
        c.selection.button_distance = val as i64
    }) {
        page.global::<TranslateSettings>()
            .set_button_distance(old_val);
    }
}

fn handle_blacklist_change(page: &SettingsWindow, val: slint::SharedString) {
    let old_val = config::snapshot().selection.blacklist;
    if !save(page, "blacklist", |c| {
        c.selection.blacklist = val.as_str().to_owned()
    }) {
        page.global::<TranslateSettings>()
            .set_blacklist(old_val.into());
    }
}

fn handle_hotkey_focus(page: &SettingsWindow, focused: bool) {
    let ts = page.global::<TranslateSettings>();
    if focused {
        ts.set_hotkey_recording(true);
        ts.set_hotkey_draft("".into());
        // 先把当前的键注销掉，否则录的时候按到它会直接触发截图
        if let Err(e) = crate::logic::hotkey::apply("") {
            log::warn!("Settings: unregister hotkey on focus failed: {e}");
        }
    } else {
        if !ts.get_hotkey_recording() {
            return;
        }
        ts.set_hotkey_recording(false);
        ts.set_hotkey_draft("".into());
        let cur = config::snapshot().screenshot.hotkey;
        if let Err(e) = crate::logic::hotkey::apply(&cur) {
            log::warn!("Settings: restore hotkey on blur failed: {e}");
        }
    }
}

fn handle_hotkey_key(
    page: &SettingsWindow,
    text: &str,
    ctrl: bool,
    shift: bool,
    alt: bool,
    meta: bool,
) -> bool {
    let ts = page.global::<TranslateSettings>();
    let Some(accel) = crate::logic::hotkey::accelerator(text, ctrl, shift, alt, meta) else {
        ts.set_hotkey_draft(crate::logic::hotkey::modifiers_only(ctrl, shift, alt, meta).into());
        return false;
    };

    ts.set_hotkey_recording(false);
    ts.set_hotkey_draft("".into());

    let old_hotkey = config::snapshot().screenshot.hotkey;
    match crate::logic::hotkey::apply(&accel) {
        Ok(()) => {
            if save(page, "hotkey", |c| c.screenshot.hotkey = accel.clone()) {
                ts.set_hotkey(accel.as_str().into());
                if !accel.is_empty() {
                    let msg = ts.invoke_show_hotkey_ok();
                    page.invoke_show_toast(msg, 1);
                }
            } else {
                let _ = crate::logic::hotkey::apply(&old_hotkey); // ignore: 保存配置失败时恢复旧热键
                ts.set_hotkey(old_hotkey.as_str().into());
            }
        }
        Err(e) => {
            let msg = ts.invoke_show_hotkey_error(e.to_string().into());
            page.invoke_show_toast(msg, 2);
            let _ = crate::logic::hotkey::apply(&old_hotkey); // ignore: 注册热键失败时恢复旧热键
            ts.set_hotkey(old_hotkey.as_str().into());
        }
    }

    true
}
