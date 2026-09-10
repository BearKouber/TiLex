use crate::config::{get, set};
use crate::screenshot::start_screenshot;
use crate::window::config_window;
use log::info;
use tauri::CustomMenuItem;
use tauri::SystemTrayEvent;
use tauri::SystemTrayMenu;
use tauri::SystemTrayMenuItem;
use tauri::SystemTraySubmenu;
use tauri::{AppHandle, Manager};

#[tauri::command]
pub fn update_tray(app_handle: tauri::AppHandle, mut language: String, mut copy_mode: String) {
    let tray_handle = app_handle.tray_handle();

    if language.is_empty() {
        language = match get("app_language") {
            Some(v) => v.as_str().unwrap().to_string(),
            None => {
                set("app_language", "en");
                "en".to_string()
            }
        };
    }
    if copy_mode.is_empty() {
        copy_mode = match get("translate_auto_copy") {
            Some(v) => v.as_str().unwrap().to_string(),
            None => {
                set("translate_auto_copy", "disable");
                "disable".to_string()
            }
        };
    }

    info!(
        "Update tray with language: {}, copy mode: {}",
        language, copy_mode
    );
    tray_handle
        .set_menu(match language.as_str() {
            "en" => tray_menu_en(),
            "zh_cn" => tray_menu_zh_cn(),
            _ => tray_menu_en(),
        })
        .unwrap();
    #[cfg(not(target_os = "linux"))]
    tray_handle
        .set_tooltip(&format!("TiLex {}", app_handle.package_info().version))
        .unwrap();

    match copy_mode.as_str() {
        "source" => tray_handle
            .get_item("copy_source")
            .set_selected(true)
            .unwrap(),
        "target" => tray_handle
            .get_item("copy_target")
            .set_selected(true)
            .unwrap(),
        "source_target" => tray_handle
            .get_item("copy_source_target")
            .set_selected(true)
            .unwrap(),
        "disable" => tray_handle
            .get_item("copy_disable")
            .set_selected(true)
            .unwrap(),
        _ => {}
    }
}

pub fn tray_event_handler<'a>(app: &'a AppHandle, event: SystemTrayEvent) {
    match event {
        #[cfg(target_os = "windows")]
        SystemTrayEvent::LeftClick { .. } => on_tray_click(),
        SystemTrayEvent::MenuItemClick { id, .. } => match id.as_str() {
            "copy_source" => on_auto_copy_click(app, "source"),
            "copy_target" => on_auto_copy_click(app, "target"),
            "copy_source_target" => on_auto_copy_click(app, "source_target"),
            "copy_disable" => on_auto_copy_click(app, "disable"),
            "screenshot" => start_screenshot(),
            "config" => on_config_click(),
            "view_log" => on_view_log_click(app),
            "restart" => on_restart_click(app),
            "quit" => on_quit_click(app),
            _ => {}
        },
        _ => {}
    }
}

#[cfg(target_os = "windows")]
fn on_tray_click() {
    config_window();
}
fn on_auto_copy_click(app: &AppHandle, mode: &str) {
    info!("Set copy mode to: {}", mode);
    set("translate_auto_copy", mode);
    app.emit_all("translate_auto_copy_changed", mode).unwrap();
    update_tray(app.app_handle(), "".to_string(), mode.to_string());
}
fn on_config_click() {
    config_window();
}

fn on_view_log_click(app: &AppHandle) {
    use tauri::api::path::app_log_dir;
    let log_path = app_log_dir(&app.config()).unwrap();
    tauri::api::shell::open(&app.shell_scope(), log_path.to_str().unwrap(), None).unwrap();
}
fn on_restart_click(app: &AppHandle) {
    info!("============== Restart App ==============");
    app.restart();
}
fn on_quit_click(app: &AppHandle) {
    info!("============== Quit App ==============");
    app.exit(0);
}

fn tray_menu_en() -> tauri::SystemTrayMenu {
    let copy_source = CustomMenuItem::new("copy_source", "Source");
    let copy_target = CustomMenuItem::new("copy_target", "Target");
    let copy_source_target = CustomMenuItem::new("copy_source_target", "Source+Target");
    let copy_disable = CustomMenuItem::new("copy_disable", "Disable");
    let screenshot = CustomMenuItem::new("screenshot", "Screenshot OCR");
    let config = CustomMenuItem::new("config", "Config");
    let view_log = CustomMenuItem::new("view_log", "View Log");
    let restart = CustomMenuItem::new("restart", "Restart");
    let quit = CustomMenuItem::new("quit", "Quit");
    SystemTrayMenu::new()
        .add_submenu(SystemTraySubmenu::new(
            "Auto Copy",
            SystemTrayMenu::new()
                .add_item(copy_source)
                .add_item(copy_target)
                .add_item(copy_source_target)
                .add_native_item(SystemTrayMenuItem::Separator)
                .add_item(copy_disable),
        ))
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(screenshot)
        .add_item(config)
        .add_item(view_log)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(restart)
        .add_item(quit)
}

fn tray_menu_zh_cn() -> tauri::SystemTrayMenu {
    let copy_source = CustomMenuItem::new("copy_source", "原文");
    let copy_target = CustomMenuItem::new("copy_target", "译文");

    let copy_source_target = CustomMenuItem::new("copy_source_target", "原文+译文");
    let copy_disable = CustomMenuItem::new("copy_disable", "关闭");
    let screenshot = CustomMenuItem::new("screenshot", "截图翻译");
    let config = CustomMenuItem::new("config", "偏好设置");
    let restart = CustomMenuItem::new("restart", "重启应用");
    let view_log = CustomMenuItem::new("view_log", "查看日志");
    let quit = CustomMenuItem::new("quit", "退出");
    SystemTrayMenu::new()
        .add_submenu(SystemTraySubmenu::new(
            "自动复制",
            SystemTrayMenu::new()
                .add_item(copy_source)
                .add_item(copy_target)
                .add_item(copy_source_target)
                .add_native_item(SystemTrayMenuItem::Separator)
                .add_item(copy_disable),
        ))
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(screenshot)
        .add_item(config)
        .add_item(view_log)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(restart)
        .add_item(quit)
}
