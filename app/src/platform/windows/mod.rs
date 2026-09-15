use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Duration;

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HANDLE, HWND, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
    DwmSetWindowAttribute,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, CreateEventW, CreateMutexW, GetCurrentThreadId, INFINITE, SetEvent,
    WaitForSingleObject,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    ASFW_ANY, AllowSetForegroundWindow, GetForegroundWindow, GetWindowThreadProcessId, IsIconic,
    SW_RESTORE, SW_SHOWNORMAL, SetForegroundWindow, ShowWindow,
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
    attach_result_window, hide_result_window, monitor_at, move_result_window,
    result_window_focused, show_result_window,
};

mod tts;
pub use tts::{speak, stop_speaking};

mod proxy;
pub use proxy::system_proxy;

/// 按会话区分（`Local\`）：同一台机器不同用户各跑各的。
const INSTANCE_MUTEX: PCWSTR = w!("Local\\TiLex.Instance");
const ACTIVATE_EVENT: PCWSTR = w!("Local\\TiLex.OpenSettings");

/// 激活事件的句柄（`HANDLE` 不是 Send，按整数存）。进程内从不关闭。
static EVENT: AtomicIsize = AtomicIsize::new(0);

pub fn data_dir() -> Result<PathBuf, Error> {
    let appdata =
        std::env::var_os("APPDATA").ok_or_else(|| Error::Platform("APPDATA is not set".into()))?;
    Ok(PathBuf::from(appdata).join("TiLex"))
}

pub fn claim_single_instance(wait: Duration) -> Result<bool, Error> {
    // 先建事件再抢锁：第二个实例紧接着启动时事件对象已经存在，它的 SetEvent 不会丢。
    // SAFETY: 常量名字符串；返回的句柄归本进程，不关闭（随进程结束释放）。
    let event = unsafe { CreateEventW(None, false, false, ACTIVATE_EVENT) }.map_err(win)?;
    // SAFETY: 同上。
    let mutex = unsafe { CreateMutexW(None, false, INSTANCE_MUTEX) }.map_err(win)?;
    let ms = u32::try_from(wait.as_millis()).unwrap_or(INFINITE - 1);
    // SAFETY: mutex 是上面刚拿到的有效句柄。主线程拿到所有权，进程退出时系统释放（重启的新实例得到 WAIT_ABANDONED）。
    let waited = unsafe { WaitForSingleObject(mutex, ms) };
    if waited == WAIT_OBJECT_0 || waited == WAIT_ABANDONED {
        EVENT.store(event.0 as isize, Ordering::Relaxed);
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
    // SAFETY: event 是有效句柄。
    unsafe { SetEvent(event) }.map_err(win)?;
    Ok(false)
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

pub fn open_path(path: &Path) -> Result<(), Error> {
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
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

pub fn bring_to_front(window: &slint::Window) -> Result<(), Error> {
    let hwnd = hwnd(window)?;
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
