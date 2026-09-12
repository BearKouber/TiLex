use crate::config::get;
use crate::screenshot::start_screenshot;
use crate::window::config_window;
use log::info;
use tauri::CustomMenuItem;
use tauri::SystemTrayEvent;
use tauri::SystemTrayMenu;
use tauri::SystemTrayMenuItem;
use tauri::{AppHandle, Manager};

#[tauri::command(async)]
pub fn update_tray(app_handle: tauri::AppHandle, language: String) {
    let tray_handle = app_handle.tray_handle();
    let language = if language.is_empty() {
        get("app_language")
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "en".into())
    } else {
        language
    };
    info!("Update tray language: {}", language);
    tray_handle
        .set_menu(match language.as_str() {
            "zh_cn" => tray_menu_zh_cn(),
            _ => tray_menu_en(),
        })
        .unwrap();
    #[cfg(not(target_os = "linux"))]
    tray_handle
        .set_tooltip(&format!("TiLex {}", app_handle.package_info().version))
        .unwrap();
}

pub fn tray_event_handler<'a>(app: &'a AppHandle, event: SystemTrayEvent) {
    match event {
        #[cfg(target_os = "windows")]
        SystemTrayEvent::LeftClick { .. } => on_tray_click(),
        SystemTrayEvent::MenuItemClick { id, .. } => match id.as_str() {
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
    let screenshot = CustomMenuItem::new("screenshot", "Screenshot OCR");
    let config = CustomMenuItem::new("config", "Config");
    let view_log = CustomMenuItem::new("view_log", "View Log");
    let restart = CustomMenuItem::new("restart", "Restart");
    let quit = CustomMenuItem::new("quit", "Quit");
    SystemTrayMenu::new()
        .add_item(screenshot)
        .add_item(config)
        .add_item(view_log)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(restart)
        .add_item(quit)
}

fn tray_menu_zh_cn() -> tauri::SystemTrayMenu {
    let screenshot = CustomMenuItem::new("screenshot", "截图翻译");
    let config = CustomMenuItem::new("config", "偏好设置");
    let restart = CustomMenuItem::new("restart", "重启应用");
    let view_log = CustomMenuItem::new("view_log", "查看日志");
    let quit = CustomMenuItem::new("quit", "退出");
    SystemTrayMenu::new()
        .add_item(screenshot)
        .add_item(config)
        .add_item(view_log)
        .add_native_item(SystemTrayMenuItem::Separator)
        .add_item(restart)
        .add_item(quit)
}
