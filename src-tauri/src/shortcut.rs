// 全项目唯一的全局快捷键：截图翻译。
//
// 批次 4 把原来那一整套快捷键（划词 / 输入 / OCR 识别 / OCR 翻译四个）删干净了，
// 这里是重新长出来的第一个，所以刻意只支持一个键，没有「按名字管理多个」那层。

use crate::config::get;
use crate::screenshot::start_screenshot;
use crate::APP;
use log::{info, warn};
use tauri::GlobalShortcutManager;

const KEY: &str = "hotkey_screenshot";

// ponytail: 全局只有这一个键，所以换键直接 unregister_all，不用记上一次注册了
// 什么。真加第二个快捷键时改成按键名单独注销。
fn apply(shortcut: &str) -> Result<(), String> {
    let app = APP.get().ok_or("APP 还没初始化")?;
    let mut mgr = app.global_shortcut_manager();
    let _ = mgr.unregister_all();
    if shortcut.is_empty() {
        return Ok(());
    }
    mgr.register(shortcut, start_screenshot)
        .map_err(|e| e.to_string())
}

/// 启动时把配置里存着的那个键装上。装不上只记日志：多半是被别的软件占了，
/// 不能因为这个把启动流程搞挂。
pub fn init_shortcut() {
    let Some(key) = get(KEY).and_then(|v| v.as_str().map(str::to_string)) else {
        return;
    };
    if key.is_empty() {
        return;
    }
    match apply(&key) {
        Ok(()) => info!("Shortcut: registered {}", key),
        Err(e) => warn!("Shortcut: {} 注册失败：{}", key, e),
    }
}

/// 设置界面改完快捷键调这个。传空串就只注销，不再注册。
/// 键被别的软件占了会在这里报错，前端直接把错误弹给用户。
#[tauri::command(async)]
pub fn register_shortcut(shortcut: String) -> Result<(), String> {
    apply(&shortcut)
}
