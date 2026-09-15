//! 界面层：每个窗口一个文件，把 Slint 回调接到 logic / platform。只在 UI 线程上运行。

pub mod pop_button;
pub mod settings;
pub mod tray;

/// 选 Slint 后端并装 winit 窗口属性钩子。必须在建第一个组件之前调用。
/// 钩子在组件的 `new()` 里同步调用；只有浮标（`pop_button::CREATING`）建成不激活的窗口，
/// 否则 winit 第一次显示时会 SW_SHOW 激活它、抢走用户正在用的窗口的焦点。
pub fn select_backend() -> Result<(), crate::error::Error> {
    slint::BackendSelector::new()
        .with_winit_window_attributes_hook(|attributes| {
            if pop_button::CREATING.get() {
                attributes.with_active(false)
            } else {
                attributes
            }
        })
        .select()?;
    Ok(())
}

/// 切换界面语言（`logic::config::LANGUAGES` 里的值）。所有窗口和托盘菜单立即重新翻译。
/// 必须在第一个 Slint 组件建好之后调用。
pub fn apply_language(language: &str) {
    if let Err(e) = slint::select_bundled_translation(language) {
        log::warn!("UI: select translation {language:?} failed: {e}");
    }
}
