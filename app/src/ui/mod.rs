//! 界面层：每个窗口一个文件，把 Slint 回调接到 logic / platform。只在 UI 线程上运行。

pub mod settings;
pub mod tray;

/// 切换界面语言（`logic::config::LANGUAGES` 里的值）。所有窗口和托盘菜单立即重新翻译。
/// 必须在第一个 Slint 组件建好之后调用。
pub fn apply_language(language: &str) {
    if let Err(e) = slint::select_bundled_translation(language) {
        log::warn!("UI: select translation {language:?} failed: {e}");
    }
}
