//! 托盘：设置 / 查看日志 / 重启 / 退出。菜单在 `ui/tray.slint`。

use crate::error::Error;
use crate::slint_ui::Tray;
use crate::{logger, platform};

/// 建托盘。调用方持有返回值直到退出：drop 时图标从通知区域移除。
pub fn create() -> Result<Tray, Error> {
    let tray = Tray::new()?;
    tray.on_open_settings(super::settings::open);
    tray.on_open_logs(|| {
        let Some(dir) = logger::dir() else { return };
        if let Err(e) = platform::open_path(dir) {
            log::warn!("Tray: open log folder failed: {e}");
        }
    });
    tray.on_restart(|| match platform::restart() {
        Ok(()) => quit(),
        Err(e) => log::error!("Tray: restart failed: {e}"),
    });
    tray.on_quit(quit);
    Ok(tray)
}

fn quit() {
    log::info!("Tray: quit");
    if let Err(e) = slint::quit_event_loop() {
        log::error!("Tray: quit event loop failed: {e}");
    }
}
