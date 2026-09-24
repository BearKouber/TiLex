use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::Duration;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::platform::windows::WindowExtWindows;
use windows::Win32::Foundation::{BOOL, HANDLE, HWND, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, DeleteObject, SetWindowRgn};
use windows::Win32::System::Threading::{
    AttachThreadInput, CreateEventW, CreateMutexW, GetCurrentThreadId, INFINITE, ResetEvent,
    SetEvent, WaitForSingleObject,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, GetForegroundWindow, GetWindowThreadProcessId,
    HWND_NOTOPMOST, HWND_TOPMOST, IsIconic, SW_RESTORE, SW_SHOWNORMAL, SWP_NOMOVE, SWP_NOSIZE,
    SetForegroundWindow, SetWindowPos, ShowWindow,
};
use windows::core::{PCWSTR, w};

use crate::error::Error;

mod selection;
pub use selection::{attach_selection_button, engage_selection, start_selection};

pub fn copy_text(text: &str) -> Result<(), Error> {
    if selection::write_text(text) {
        Ok(())
    } else {
        Err(Error::Platform("clipboard write failed".into()))
    }
}

mod result_window;
pub use result_window::{
    attach_overlay_window, attach_result_window, cursor_pos, hide_overlay_window,
    hide_result_window, monitor_at, move_result_window, result_window_focused, show_overlay_window,
    show_result_window,
};

mod tts;
pub use tts::{speak, stop_speaking};

mod proxy;
pub use proxy::system_proxy;

mod autostart;
pub use autostart::{autostart_enabled, set_autostart};

mod screenshot;
pub use screenshot::capture_screen;

mod ocr;
pub use ocr::{wechat_ocr, wechat_ocr_status};

/// 按会话区分（`Local\`）：同一台机器不同用户各跑各的。
const INSTANCE_MUTEX: PCWSTR = w!("Local\\TiLex.Instance");
const ACTIVATE_EVENT: PCWSTR = w!("Local\\TiLex.OpenSettings");
const ACTIVATE_ACK_EVENT: PCWSTR = w!("Local\\TiLex.OpenSettingsAck");

/// 激活事件的句柄（`HANDLE` 不是 Send，按整数存）。进程内从不关闭。
static EVENT: AtomicIsize = AtomicIsize::new(0);
static ACK_EVENT: AtomicIsize = AtomicIsize::new(0);

pub fn data_dir() -> Result<PathBuf, Error> {
    let appdata =
        std::env::var_os("APPDATA").ok_or_else(|| Error::Platform("APPDATA is not set".into()))?;
    Ok(PathBuf::from(appdata).join("TiLex"))
}

pub fn cache_dir() -> Result<PathBuf, Error> {
    let localappdata = std::env::var_os("LOCALAPPDATA")
        .ok_or_else(|| Error::Platform("LOCALAPPDATA is not set".into()))?;
    Ok(PathBuf::from(localappdata).join("TiLex").join("cache"))
}

pub fn claim_single_instance(wait: Duration) -> Result<bool, Error> {
    // 先建事件再抢锁：第二个实例紧接着启动时事件对象已经存在，它的 SetEvent 不会丢。
    // SAFETY: 常量名字符串；返回的句柄归本进程，不关闭（随进程结束释放）。
    let event = unsafe { CreateEventW(None, false, false, ACTIVATE_EVENT) }.map_err(win)?;
    // SAFETY: ACK 事件同上。
    let ack_event = unsafe { CreateEventW(None, false, false, ACTIVATE_ACK_EVENT) }.map_err(win)?;
    // SAFETY: 同上。
    let mutex = unsafe { CreateMutexW(None, false, INSTANCE_MUTEX) }.map_err(win)?;
    let ms = u32::try_from(wait.as_millis()).unwrap_or(INFINITE - 1);
    // SAFETY: mutex 是上面刚拿到的有效句柄。主线程拿到所有权，进程退出时系统释放（重启的新实例得到 WAIT_ABANDONED）。
    let waited = unsafe { WaitForSingleObject(mutex, ms) };
    if waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED {
        EVENT.store(event.0 as isize, Ordering::Relaxed);
        ACK_EVENT.store(ack_event.0 as isize, Ordering::Relaxed);
        return Ok(true);
    }
    if waited != WAIT_TIMEOUT {
        return Err(Error::Platform(format!(
            "wait for instance mutex: {:?}",
            waited
        )));
    }
    // 已有实例：本进程是用户刚启动的，有前台权；让出去，老实例的设置窗口才能到前台（前台锁）。
    // SAFETY: 无指针参数。
    let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) }; // ignore: 失败只是窗口可能出现在后面
    // SAFETY: ack_event 是有效句柄。第二实例在 SetEvent(activate) 之前先 ResetEvent(ack)。
    let _ = unsafe { ResetEvent(ack_event) }; // ignore: 清理旧信号失败不致命
    // SAFETY: event 是有效句柄。
    unsafe { SetEvent(event) }.map_err(win)?;
    // SAFETY: ack_event 是有效句柄。等待老实例唤起设置窗完成，最长 2000ms；超时照常退出。
    let _ = unsafe { WaitForSingleObject(ack_event, 2000) }; // ignore: 等待超时或失败照常退出
    Ok(false)
}

pub fn ack_activation() {
    let raw = ACK_EVENT.load(Ordering::Relaxed);
    if raw != 0 {
        let event = HANDLE(raw as *mut c_void);
        // SAFETY: event 在 claim_single_instance 中创建，句柄有效。
        // ignore: 无第二实例等待时信号保留，由下一次第二实例在激活前重置
        let _ = unsafe { SetEvent(event) };
    }
}

pub fn listen_activation(on_activate: impl Fn() + Send + 'static) -> Result<(), Error> {
    let raw = EVENT.load(Ordering::Relaxed);
    if raw == 0 {
        return Err(Error::Platform("single instance not claimed".into()));
    }
    std::thread::Builder::new()
        .name("instance-listener".into())
        .spawn(move || {
            let event = HANDLE(raw as *mut c_void);
            loop {
                // SAFETY: 事件句柄在 claim 里创建，进程内从不关闭。
                let waited = unsafe { WaitForSingleObject(event, INFINITE) };
                if waited != WAIT_OBJECT_0 {
                    log::error!("Instance: wait for activation failed: {waited:?}");
                    return;
                }
                on_activate();
            }
        })?;
    Ok(())
}

fn shell_open_wide(wide: &[u16]) -> Result<(), Error> {
    // SAFETY: wide 以 0 结尾，调用期间有效；其余指针参数为空。
    let result = unsafe {
        ShellExecuteW(
            HWND::default(),
            w!("open"),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    // 文档约定：返回值大于 32 表示成功。
    let code = result.0 as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(Error::Platform(format!("ShellExecuteW failed: {code}")))
    }
}

pub fn open_path(path: &Path) -> Result<(), Error> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    shell_open_wide(&wide)
}

pub fn open_url(url: &str) -> Result<(), Error> {
    let wide: Vec<u16> = url.encode_utf16().chain(Some(0)).collect();
    shell_open_wide(&wide)
}

/// `DWMWA_WINDOW_CORNER_PREFERENCE` 是 Win11 22000 才有的。这台机器上用不了（= Win10）时置位，
/// 圆角改由 [`round_region`] 裁窗口区域。
///
/// 这是**系统属性不是窗口属性**，几个无边框窗口共用一份：谁先试出来谁置位。
static DWM_ROUNDING_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

/// DWM 圆角这次没吃上，后面改用窗口区域裁。
fn mark_dwm_rounding_unavailable() {
    DWM_ROUNDING_UNAVAILABLE.store(true, Ordering::SeqCst);
}

/// Win10 上把窗口裁成圆角矩形（DWM 圆角能用时直接返回，不动它的平滑圆角）。
/// 裁掉四个角的同时，系统沿直角边缘画的那圈强调色也一起没了 —— 它画在窗口区域最外圈，
/// 区域裁掉就不再合成（Win10 22H2 实机验证过）。代价是边缘没有抗锯齿，比 DWM 的圆角糙一点。
///
/// 窗口区域不跟着 `SetWindowPos` 走，**每次尺寸变化后都要重设**，
/// 和 `DWMWCP_*` 的遮罩要重算是同一个道理。
/// `w`/`h_px` 是窗口矩形的物理像素尺寸，`radius` 是逻辑像素半径（按窗口 DPI 换算）。
fn round_region(h: HWND, w: i32, h_px: i32, radius: f32) {
    if !DWM_ROUNDING_UNAVAILABLE.load(Ordering::SeqCst) || w <= 0 || h_px <= 0 {
        return;
    }
    // SAFETY: h 是活着的窗口，无指针参数。
    let dpi = unsafe { GetDpiForWindow(h) };
    let scale = if dpi == 0 { 1.0 } else { dpi as f32 / 96.0 };
    // CreateRoundRectRgn 收的是椭圆的**直径**，不是半径。
    let d = (radius * scale).round() as i32 * 2;
    // SAFETY: 纯计算，不碰指针。右下角是 exclusive，+1 才盖得满整个窗口。
    let rgn = unsafe { CreateRoundRectRgn(0, 0, w + 1, h_px + 1, d, d) };
    if rgn.is_invalid() {
        log::warn!("round_region: CreateRoundRectRgn failed");
        return;
    }
    // SAFETY: rgn 刚建好且有效；h 是活着的窗口。
    // 成功时 region 的所有权转给系统，**不能**再 DeleteObject。
    if unsafe { SetWindowRgn(h, rgn, BOOL::from(true)) } == 0 {
        log::warn!("round_region: SetWindowRgn failed");
        // SAFETY: 失败时所有权还在我们手上，不删就泄漏。
        // ignore: 删不掉也只是漏一个 region 对象
        let _ = unsafe { DeleteObject(rgn) };
    }
}

pub fn round_corners(window: &slint::Window) -> Result<(), Error> {
    let hwnd = hwnd(window)?;
    let pref = DWMWCP_ROUND;
    // SAFETY: hwnd 来自活着的 Slint 窗口；pref 在调用期间有效，长度与类型一致。
    unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const pref).cast::<c_void>(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
    }
    .map_err(|e| Error::Platform(format!("DWM corner preference: {e}")))
}

/// 给无边框设置窗口应用 Win11 圆角和窗口阴影。
/// 原生窗口未就绪时返回 `Error::Platform`（调用方通过 Timer 重试）。
pub fn style_frameless_window(window: &slint::Window) -> Result<(), Error> {
    // 阴影：winit undecorated shadow
    window
        .with_winit_window(|w| {
            w.set_undecorated_shadow(true);
        })
        .ok_or_else(|| Error::Platform("winit window not ready".into()))?;

    // 圆角：Win11 DWM 圆角；Win10 返回错误只 log::info!，不算整体失败
    if let Err(e) = round_corners(window) {
        log::info!("Settings: round corners skipped or unsupported: {e}");
    }

    Ok(())
}

pub fn bring_to_front(window: &slint::Window) -> Result<(), Error> {
    let hwnd = hwnd(window)?;
    if force_foreground(hwnd) {
        return Ok(());
    }
    // 抢不到前台时先置顶再取消置顶，至少把窗口提到其它窗口上面，再试一次。
    // 只给普通窗口（设置窗）用：结果窗、遮罩本来就是 TOPMOST，走这一步会被取消置顶。
    // SAFETY: hwnd 来自活着的 Slint 窗口；无指针参数。
    unsafe {
        let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE); // ignore: 提层失败就只剩下面的重试
        let _ = SetWindowPos(hwnd, HWND_NOTOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE); // ignore: 同上
    }
    if force_foreground(hwnd) {
        Ok(())
    } else {
        Err(Error::Platform("SetForegroundWindow refused".into()))
    }
}

pub(super) fn force_foreground(hwnd: HWND) -> bool {
    // SAFETY: hwnd 来自活着的窗口；其余调用无指针参数。
    unsafe {
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE); // ignore: 返回值是"之前是否可见"，不是错误
        }
        // 前台锁：把输入队列临时挂到当前前台线程上再抢前台，挂完立刻摘（旧版 force_foreground）。
        let fg = GetForegroundWindow();
        let fg_thread = if fg.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        let me = GetCurrentThreadId();
        let attached =
            fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, true).as_bool();
        let ok = SetForegroundWindow(hwnd).as_bool();
        if attached {
            let _ = AttachThreadInput(me, fg_thread, false); // ignore: 摘不掉也只是两个输入队列继续共享到对方线程退出
        }
        ok
    }
}

pub fn is_foreground(window: &slint::Window) -> bool {
    let Ok(h) = hwnd(window) else {
        return false;
    };
    // SAFETY: 无指针参数。
    let fg = unsafe { GetForegroundWindow() };
    fg == h
}

pub(super) fn hwnd(window: &slint::Window) -> Result<HWND, Error> {
    let handle = window.window_handle();
    let raw = handle
        .window_handle()
        .map_err(|e| Error::Platform(format!("window handle: {e}")))?
        .as_raw();
    match raw {
        RawWindowHandle::Win32(h) => Ok(HWND(h.hwnd.get() as *mut c_void)),
        _ => Err(Error::Platform("not a Win32 window".into())),
    }
}

fn win(e: windows::core::Error) -> Error {
    Error::Platform(e.to_string())
}
