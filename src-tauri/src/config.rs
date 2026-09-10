use crate::{error::Error, APP};
use dirs::config_dir;
use log::{info, warn};
use serde_json::{json, Value};
use std::sync::Mutex;
use tauri::{Manager, Wry};
use tauri_plugin_store::{Store, StoreBuilder};

pub struct StoreWrapper(pub Mutex<Store<Wry>>);

pub fn init_config(app: &mut tauri::App) {
    let config_path = config_dir().unwrap();
    let config_path = config_path.join(app.config().tauri.bundle.identifier.clone());
    let config_path = config_path.join("config.json");
    info!("Load config from: {:?}", config_path);
    let mut store = StoreBuilder::new(app.handle(), config_path).build();

    match store.load() {
        Ok(_) => info!("Config loaded"),
        Err(e) => {
            warn!("Config load error: {:?}", e);
            info!("Config not found, creating new config");
        }
    }
    app.manage(StoreWrapper(Mutex::new(store)));
    let _ = check_service_available();
}

fn check_available(list: Vec<String>, builtin: Vec<&str>, key: &str) {
    // 实例 key 形如 `google@ab12cd`，@ 前面才是服务名。
    let new_list: Vec<String> = list
        .iter()
        .filter(|s| builtin.contains(&s.split('@').next().unwrap_or("")))
        .cloned()
        .collect();
    if new_list.len() != list.len() {
        set(key, new_list);
    }
}

pub fn check_service_available() -> Result<(), Error> {
    // 这两份清单是「哪些服务名还活着」的唯一判据，不在里面的实例会被下面直接剪掉。
    // 加服务 / 删服务时必须同步改，否则新服务重启一次就消失（见 RECOGNIZE_SERVICES
    // 与 src/services/translate/index.jsx 的 export 列表，两边一一对应）。
    let builtin_recognize_list: Vec<&str> = vec!["wechat", "umi"];
    let builtin_translate_list: Vec<&str> =
        vec!["baidu", "bing", "deepl", "google", "transmart", "ai"];

    if let Some(recognize_service_list) = get("recognize_service_list") {
        let recognize_service_list: Vec<String> = serde_json::from_value(recognize_service_list)?;
        check_available(
            recognize_service_list,
            builtin_recognize_list,
            "recognize_service_list",
        );
    }
    if let Some(translate_service_list) = get("translate_service_list") {
        let translate_service_list: Vec<String> = serde_json::from_value(translate_service_list)?;
        check_available(
            translate_service_list,
            builtin_translate_list,
            "translate_service_list",
        );
    }
    Ok(())
}

pub fn get(key: &str) -> Option<Value> {
    let state = APP.get().unwrap().state::<StoreWrapper>();
    let store = state.0.lock().unwrap();
    match store.get(key) {
        Some(value) => Some(value.clone()),
        None => None,
    }
}

pub fn set<T: serde::ser::Serialize>(key: &str, value: T) {
    let state = APP.get().unwrap().state::<StoreWrapper>();
    let mut store = state.0.lock().unwrap();
    store.insert(key.to_string(), json!(value)).unwrap();
    store.save().unwrap();
}

pub fn is_first_run() -> bool {
    let state = APP.get().unwrap().state::<StoreWrapper>();
    let store = state.0.lock().unwrap();
    store.is_empty()
}
