//! 平台接口。每个函数在这里写一次签名，函数体转调 `imp`（design §2.11，不用 trait）。
//! `#[cfg]` 只允许出现在 `platform/` 下。

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::error::Error;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as imp;

pub mod process;

/// 托盘"重启"拉起的新进程带这个参数，启动时多等旧实例退出。
pub const RESTART_FLAG: &str = "--restart";

/// 数据目录：Windows `%APPDATA%\TiLex`，macOS `~/Library/Application Support/TiLex`（design §2.2）。
/// 只拼路径，不创建。
pub fn data_dir() -> Result<PathBuf, Error> {
    imp::data_dir()
}

/// 抢单实例。拿到返回 `Ok(true)`；已有实例在跑时通知它打开设置窗口，返回 `Ok(false)`，调用方应直接退出。
/// `wait`：等已有实例退出的时间（重启时用，平时是 0）。
pub fn claim_single_instance(wait: Duration) -> Result<bool, Error> {
    imp::claim_single_instance(wait)
}

/// `claim_single_instance` 成功后调用：起一个后台线程等别的实例的通知，每次调 `on_activate`（在那个线程上）。
/// Windows 上 claim 与 listen 之间到达的通知不会丢；macOS 上 socket 建好之前的会丢。
pub fn listen_activation(on_activate: impl Fn() + Send + 'static) -> Result<(), Error> {
    imp::listen_activation(on_activate)
}

/// 用系统默认方式打开文件或文件夹（资源管理器 / 访达）。
pub fn open_path(path: &Path) -> Result<(), Error> {
    imp::open_path(path)
}

/// 拉起一个新的自己（带 `RESTART_FLAG`）。调用方随后退出事件循环。
pub fn restart() -> Result<(), Error> {
    process::spawn(&std::env::current_exe()?, &[RESTART_FLAG.into()])
}

/// 给无边框窗口加系统圆角。Win11 走 DWM；Win10 不支持，返回错误由调用方决定是否显示。
/// 原生窗口必须已经存在：`show()` 之后还要等事件循环跑起来（winit 在 resumed / about_to_wait
/// 里才建窗口），否则拿不到句柄，返回 `Error::Platform`。
pub fn round_corners(window: &slint::Window) -> Result<(), Error> {
    imp::round_corners(window)
}

/// 把已经显示的窗口拉到最前面并激活（最小化的先还原）。`show()` 对已显示的窗口什么都不做，
/// 而 Windows 的前台锁会静默拒绝后台进程的 `SetForegroundWindow`，所以要走平台层。
pub fn bring_to_front(window: &slint::Window) -> Result<(), Error> {
    imp::bring_to_front(window)
}
