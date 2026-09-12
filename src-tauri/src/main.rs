// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cmd;
mod config;
mod error;
mod lang_detect;
mod ocr;
mod pop_button;
mod screenshot;
mod shortcut;
mod tray;
mod window;

use cmd::*;
use config::*;
use lang_detect::*;
use ocr::*;
use pop_button::*;
use screenshot::*;
use shortcut::*;
use log::info;
use once_cell::sync::OnceCell;
use std::sync::Mutex;
use tauri::Manager;
use tauri_plugin_log::LogTarget;
use tray::*;
use window::config_window;

// Global AppHandle
pub static APP: OnceCell<tauri::AppHandle> = OnceCell::new();

// Text to be translated
pub struct StringWrapper(pub Mutex<String>);

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("PANIC: {:?}", info);
        eprintln!("{}", msg);
        log::error!("{}", msg);
    }));
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|_app, _, _cwd| {
            config_window();
        }))
        .plugin(
            tauri_plugin_log::Builder::default()
                .targets([LogTarget::LogDir, LogTarget::Stdout])
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_sql::Builder::default().build())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_fs_watch::init())
        .system_tray(tauri::SystemTray::new())
        .setup(|app| {
            info!("============== Start App ==============");
            #[cfg(target_os = "macos")]
            {
                app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                let trusted =
                    macos_accessibility_client::accessibility::application_is_trusted_with_prompt();
                info!("MacOS Accessibility Trusted: {}", trusted);
            }
            // Global AppHandle
            APP.get_or_init(|| app.handle());
            // Init Config
            info!("Init Config Store");
            init_config(app);
            // 启动要静默：只有第一次运行才把配置窗口摆出来，之后一律留在托盘。
            // 开机自启的场景尤其不能弹——每次开机都糊一个 800x600 在脸上。
            // 想看设置有两条路：托盘菜单，或者再点一次 exe（单例回调会开窗）。
            if is_first_run() {
                info!("First Run, opening config window");
                config_window();
            }
            app.manage(StringWrapper(Mutex::new("".to_string())));
            // Update Tray Menu
            update_tray(app.app_handle(), "".to_string(), "".to_string());
            if let Some(engine) = get("translate_detect_engine") {
                if engine.as_str().unwrap() == "local" {
                    init_lang_detect();
                }
            }
            start_pop_button();
            init_shortcut();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            reload_store,
            get_text,
            open_devtools,
            update_tray,
            lang_detect,
            pop_button_translate,
            ocr_image,
            ocr_status,
            crop_region,
            show_pop_result,
            screenshot_current,
            screenshot_is_current,
            screenshot_cancel,
            screenshot_overlay,
            screenshot_publish,
            register_shortcut
        ])
        .on_system_tray_event(tray_event_handler)
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        // 窗口关闭不退出
        .run(|_app_handle, event| match event {
            tauri::RunEvent::ExitRequested { api, .. } => {
                info!("RunEvent::ExitRequested received, preventing exit");
                api.prevent_exit();
            }
            tauri::RunEvent::Exit => {
                info!("RunEvent::Exit received");
            }
            _ => {}
        });
}
