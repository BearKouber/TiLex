//! 界面层：每个窗口一个文件，把 Slint 回调接到 logic / platform。只在 UI 线程上运行。

use std::cell::Cell;

pub mod entry_view;
pub mod pop_button;
pub mod pop_result;
pub mod settings;
pub mod tray;

thread_local! {
    /// 正在建不激活的窗口（浮标、结果浮窗）。
    /// `select_backend` 装的 winit 属性钩子只在这时加 `with_active(false)`。
    pub(crate) static CREATING_INACTIVE: Cell<bool> = const { Cell::new(false) };
}

/// 选 Slint 后端并装 winit 窗口属性钩子。必须在建第一个组件之前调用。
/// 钩子在组件的 `new()` 里同步调用；只有浮标与浮窗（`CREATING_INACTIVE`）建成不激活的窗口，
/// 否则 winit 第一次显示时会 SW_SHOW 激活它、抢走用户正在用的窗口的焦点。
pub fn select_backend() -> Result<(), crate::error::Error> {
    slint::BackendSelector::new()
        .with_winit_window_attributes_hook(|attributes| {
            if CREATING_INACTIVE.get() {
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

/// 根据当前配置及窗口环境计算 `ColorScheme`（配置值 + system 时问 winit）。
/// 设置窗口与结果浮窗共用此逻辑。
pub fn resolve_color_scheme(window: &slint::Window) -> slint::language::ColorScheme {
    use slint::language::ColorScheme;
    use slint::winit_030::WinitWindowAccessor;
    use slint::winit_030::winit::window::Theme as WinitTheme;

    let cfg = crate::logic::config::snapshot();
    match cfg.general.theme.as_str() {
        "dark" => ColorScheme::Dark,
        "light" => ColorScheme::Light,
        _ => match window.with_winit_window(|w| w.theme()).flatten() {
            Some(WinitTheme::Dark) => ColorScheme::Dark,
            Some(WinitTheme::Light) => ColorScheme::Light,
            // 拿不到系统主题时回退 Unknown：fluent 走系统默认，自绘部分因 dark 判断为 false 按亮色显示（已知限制）
            None => ColorScheme::Unknown,
        },
    }
}
