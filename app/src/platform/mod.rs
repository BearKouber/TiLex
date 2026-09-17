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

mod bypass;
pub mod geometry;
pub mod process;

use geometry::Side;

/// 托盘"重启"拉起的新进程带这个参数，启动时多等旧实例退出。
pub const RESTART_FLAG: &str = "--restart";

/// 数据目录：Windows `%APPDATA%\TiLex`，macOS `~/Library/Application Support/TiLex`（design §2.2）。
/// 只拼路径，不创建。
pub fn data_dir() -> Result<PathBuf, Error> {
    imp::data_dir()
}

/// 缓存目录：Windows `%LOCALAPPDATA%\TiLex\cache`，macOS `~/Library/Caches/TiLex`（design §2.2）。
/// OCR 的临时图落在这里。只拼路径，不创建。
pub fn cache_dir() -> Result<PathBuf, Error> {
    imp::cache_dir()
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

/// 用系统默认浏览器打开 URL。
pub fn open_url(url: &str) -> Result<(), Error> {
    imp::open_url(url)
}

/// 开机自启是否已开启（Windows 检查 Run 注册表键且指向当前 exe；macOS 返回 Unsupported）。
pub fn autostart_enabled() -> Result<bool, Error> {
    imp::autostart_enabled()
}

/// 设置开机自启（Windows 写/删 Run 注册表键；macOS 返回 Unsupported）。
pub fn set_autostart(on: bool) -> Result<(), Error> {
    imp::set_autostart(on)
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

/// 给无边框窗口应用平台原生外框样式（Win11 DWM 圆角 + 窗口阴影）。
/// 原生窗口未建好时返回 `Error::Platform`，调用方应使用 Timer 重试。
pub fn style_frameless_window(window: &slint::Window) -> Result<(), Error> {
    imp::style_frameless_window(window)
}

/// 把已经显示的窗口拉到最前面并激活（最小化的先还原）。`show()` 对已显示的窗口什么都不做，
/// 而 Windows 的前台锁会静默拒绝后台进程的 `SetForegroundWindow`，所以要走平台层。
pub fn bring_to_front(window: &slint::Window) -> Result<(), Error> {
    imp::bring_to_front(window)
}

/// 划词监听要用的设置。取词 worker 每次手势都现取（改了配置不用重启）。
pub struct SelectionSettings {
    /// 划词开关。关着时不读选区、不模拟复制、不出浮标。
    pub enabled: bool,
    /// 逗号分隔的进程名（不区分大小写，`.exe` 可省）。前台是这些进程时不出浮标。
    pub blacklist: String,
    /// 增强选中识别：读不到选区的程序里模拟一次复制，读完恢复剪贴板。
    pub force_copy: bool,
    /// 浮标在松开鼠标处的哪个方位（x 轴, y 轴）。
    pub corner: (Side, Side),
    /// 浮标离光标的间距，物理像素（配置里已夹到 0–20）。
    pub gap: i32,
}

/// 用户悬停/点击了浮标。`x`、`y` 是浮标中心，物理像素（结果浮窗以它为基准摆放）。
pub struct EngagedSelection {
    pub text: String,
    pub x: i32,
    pub y: i32,
}

/// 启动划词监听：鼠标/键盘钩子线程 + 取词 worker 线程，进程结束前不停。
/// 四个回调都在取词 worker 线程上调用，不许碰 Slint 组件：
/// - `settings`：现取设置，一次手势里会调好几次，要便宜；
/// - `accept`：读到文字后、出浮标之前问一次，返回 `false` 就不出（排除母语的挂钩点）；
/// - `engaged`：用户悬停/点击了浮标，文字交给调用方；
/// - `before_show`：在取词 worker 线程上、浮标每次显示之前调用；调用方让浮标整窗重画（Slint 软件渲染只画脏区域，DWM cloak 期间画面可能丢失导致透明空框，见 platform-windows.md §1 第 4 条）；不许碰 Slint 组件（要切回 UI 线程）。
///
/// 浮标窗口另外用 [`attach_selection_button`] 交进来；交进来之前读到的选区只是不显示。
pub fn start_selection(
    settings: impl Fn() -> SelectionSettings + Send + 'static,
    accept: impl Fn(&str) -> bool + Send + 'static,
    engaged: impl Fn(EngagedSelection) + Send + 'static,
    before_show: impl Fn() + Send + 'static,
) -> Result<(), Error> {
    imp::start_selection(
        Box::new(settings),
        Box::new(accept),
        Box::new(engaged),
        Box::new(before_show),
    )
}

type SettingsFn = Box<dyn Fn() -> SelectionSettings + Send>;
type AcceptFn = Box<dyn Fn(&str) -> bool + Send>;
type EngagedFn = Box<dyn Fn(EngagedSelection) + Send>;
type BeforeShowFn = Box<dyn Fn() + Send>;

/// 把浮标窗口交给平台层。之后它的显示、隐藏、位置只由平台层管，**调用方再也不能调它的 `show()`/`hide()`**。
/// 调用前窗口必须已经 `show()` 过一次（最好在屏幕外）；原生窗口要到事件循环之后的某一轮才有，
/// 没有时返回 `Error::Platform`，调用方用 `slint::Timer` 重试。
pub fn attach_selection_button(window: &slint::Window) -> Result<(), Error> {
    imp::attach_selection_button(window)
}

/// 浮标被悬停/点击（UI 线程调）。浮标没显示时什么都不做；重复调用只算一次。
pub fn engage_selection() {
    imp::engage_selection()
}

/// 朗读用的语音。选哪种由调用方按文字定（含汉字/假名读中文，其他读英文）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Voice {
    Chinese,
    English,
}

/// 开始朗读 `text`，立刻返回，在后台线程读。正在读别的就先停掉它。
/// `done` 在读完、被停止或中途出错后，在后台线程上恰好调用一次（UI 用它把按钮切回"朗读"）。
/// 返回 `Err` 时没有开始读，`done` 不会被调用。
pub fn speak(text: &str, voice: Voice, done: impl FnOnce() + Send + 'static) -> Result<(), Error> {
    imp::speak(text, voice, done)
}

/// 停止当前朗读；没在读就什么都不做。任何线程都能调。
pub fn stop_speaking() {
    imp::stop_speaking()
}

/// `url` 该走的系统代理（`http://host:port`），直连返回 `None`。每次请求时查，改了 Clash 设置不用重启。
/// 回环地址永远直连；只认手动 HTTP 代理，PAC 和 SOCKS 不支持（design §2.5）。
pub fn system_proxy(url: &str) -> Option<String> {
    imp::system_proxy(url)
}

/// (x, y) 所在显示器的工作区（物理像素，可为负坐标）和缩放比（DPI / 96）。
pub fn monitor_at(x: i32, y: i32) -> Option<(geometry::Rect, f32)> {
    imp::monitor_at(x, y)
}

/// 把结果浮窗交给平台层（UI 线程调）。应用无边框样式（WS_POPUP / WS_EX_TOOLWINDOW / WS_EX_TOPMOST）、
/// 设置 Win11 小圆角并初始 DWM cloak 隐藏。
/// 调用前窗口必须已经 `show()` 过一次；原生窗口尚未创建时返回 `Error::Platform`，调用方用 Timer 重试。
pub fn attach_result_window(window: &slint::Window) -> Result<(), Error> {
    imp::attach_result_window(window)
}

/// 在指定的屏幕矩形位置（物理像素）显示结果浮窗（UI 线程调）。
/// 尚未 attach 时什么都不做。为避免跨 DPI 显示器抖动，内部先移动位置再带尺寸摆放；
/// 随后解除 cloak、强拉到前台抢焦点。显示前记录当时的前台窗口（若不是浮窗自己），供隐藏时切回原程序。
pub fn show_result_window(rect: geometry::Rect) {
    imp::show_result_window(rect)
}

/// 调整结果浮窗在屏幕上的物理像素矩形位置与尺寸（UI 线程调，内容撑大或拖拽贴边时调）。
/// 尚未 attach 时什么都不做。仅调整位置尺寸（SWP_NOACTIVATE），不影响当前焦点。
pub fn move_result_window(rect: geometry::Rect) {
    imp::move_result_window(rect)
}

/// 隐藏结果浮窗（UI 线程调，DWM cloak）。
/// 尚未 attach 时什么都不做。只有当前台仍是浮窗自己时（如 Esc / 叉号关闭），才将焦点还给显示前记录的原窗口；
/// 若因失焦而隐藏，焦点已在别处，不抢还焦点。
pub fn hide_result_window() {
    imp::hide_result_window()
}

/// 查询结果浮窗当前是否拥有系统前台焦点（UI 线程或计时器调）。
/// 浮窗尚未 attach 时返回 `None`；拥有焦点返回 `Some(true)`，失去焦点返回 `Some(false)`。
pub fn result_window_focused() -> Option<bool> {
    imp::result_window_focused()
}

/// 把截图遮罩交给平台层（UI 线程调，启动时一次）。应用无边框样式、置顶，并 cloak 隐藏。
/// 之后它的显示、隐藏、位置只由平台层管，**调用方再也不能调它的 `show()`/`hide()`**：
/// 走 `ShowWindow` 会播系统的开窗缩放动画（design §1.4）。
/// 调用前窗口必须已经在屏幕外 `show()` 过一次；原生窗口尚未创建时返回 `Error::Platform`，
/// 调用方用 Timer 重试。
pub fn attach_overlay_window(window: &slint::Window) -> Result<(), Error> {
    imp::attach_overlay_window(window)
}

/// 把遮罩摆到 `rect`（物理像素，整个虚拟屏）并显示、抢焦点（UI 线程调）。
/// 尚未 attach 时什么都不做。显示前记下当时的前台窗口，隐藏时还回去。
pub fn show_overlay_window(rect: geometry::Rect) {
    imp::show_overlay_window(rect)
}

/// 隐藏遮罩并挪回屏幕外（UI 线程调，DWM cloak）。
/// 前台仍是遮罩自己时，把焦点还给显示前记下的那个窗口 —— 否则用户的键盘输入
/// 会进一个看不见的窗口，而且随后弹出的结果浮窗会把遮罩当成"原来的程序"。
pub fn hide_overlay_window() {
    imp::hide_overlay_window()
}

/// 将文本写入系统剪贴板。任何线程均可调用，若剪贴板正被占用最多会阻塞重试约 10 次（约 10×重试间隔）。
pub fn copy_text(text: &str) -> Result<(), Error> {
    imp::copy_text(text)
}

/// 抓下来的整个虚拟屏（物理像素）。`x`/`y` 是虚拟屏左上角在桌面坐标系里的位置，多屏时可以是负数。
pub struct Shot {
    pub x: i32,
    pub y: i32,
    pub pixels: slint::SharedPixelBuffer<slint::Rgba8Pixel>,
}

/// 抓整个虚拟屏。几十毫秒，**不要在 UI 线程上调**（R-5）。
pub fn capture_screen() -> Result<Shot, Error> {
    imp::capture_screen()
}

/// 微信 OCR 识别一张图，返回图里的文字。几百毫秒到几秒，**不要在 UI 线程上调**（R-5）。
/// 没装微信、没下过 OCR 插件、识别不出字都返回 `Error::Platform`，消息是给用户看的。
pub fn wechat_ocr(image: &Path) -> Result<String, Error> {
    imp::wechat_ocr(image)
}

/// 微信 OCR 能不能用。能用返回微信版本号（04 的识别服务列表要显示），不能用返回缺什么。
pub fn wechat_ocr_status() -> Result<String, Error> {
    imp::wechat_ocr_status()
}
