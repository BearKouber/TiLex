//! 结果浮窗的原生窗口管理（Win32）。
//! 启动时建好并在屏幕外 show 一次，之后显示/隐藏全部用 DWM cloak + SetWindowPos，
//! 永远不调 Slint 的 show()/hide()（platform-windows.md §1）。

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{BOOL, HWND, POINT};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_CLOAK, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUNDSMALL,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetForegroundWindow, GetSystemMetrics, GetWindowLongPtrW, HWND_TOPMOST,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
    WS_CAPTION, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_MAXIMIZEBOX, WS_MINIMIZEBOX,
    WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
};

use crate::error::Error;
use crate::platform::geometry::Rect;

static RESULT_WINDOW: AtomicIsize = AtomicIsize::new(0);
static PREV_FOREGROUND: AtomicIsize = AtomicIsize::new(0);

fn result_hwnd() -> Option<HWND> {
    let raw = RESULT_WINDOW.load(Ordering::SeqCst);
    (raw != 0).then_some(HWND(raw as *mut c_void))
}

/// (x, y) 所在显示器的工作区（物理像素，可为负坐标）和缩放比（DPI / 96）。
/// 查不到显示器时退回整个虚拟屏幕，保证不失败。
pub(super) fn work_area(x: i32, y: i32) -> (Rect, f32) {
    // SAFETY: 结构体按文档初始化 cbSize；无其他指针。
    unsafe {
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let (mut dx, mut dy) = (96u32, 96u32);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dx, &mut dy).is_err() {
            dx = 96;
        }
        let scale = dx as f32 / 96.0;
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let r = info.rcWork;
            return (
                Rect {
                    l: r.left,
                    t: r.top,
                    r: r.right,
                    b: r.bottom,
                },
                scale,
            );
        }
        let (l, t) = (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
        );
        let virtual_screen = Rect {
            l,
            t,
            r: l + GetSystemMetrics(SM_CXVIRTUALSCREEN),
            b: t + GetSystemMetrics(SM_CYVIRTUALSCREEN),
        };
        (virtual_screen, scale)
    }
}

/// (x, y) 所在显示器的工作区（物理像素，可为负坐标）和缩放比（DPI / 96）。
pub fn monitor_at(x: i32, y: i32) -> Option<(Rect, f32)> {
    Some(work_area(x, y))
}

pub fn attach_result_window(window: &slint::Window) -> Result<(), Error> {
    let h = super::hwnd(window)?;
    apply_styles(h);
    // Win11 小圆角
    let pref = DWMWCP_ROUNDSMALL;
    // SAFETY: h 是活着的窗口；pref 在调用期间有效，长度与类型一致。
    if let Err(e) = unsafe {
        DwmSetWindowAttribute(
            h,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const pref).cast::<c_void>(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
    } {
        log::debug!("PopResult: no rounded corners: {e}");
    }
    cloak(h, true);
    // SAFETY: 只刷新样式，不动位置、大小、激活。
    // ignore: 刷新失败时样式照样生效
    let _ = unsafe {
        SetWindowPos(
            h,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        )
    };
    RESULT_WINDOW.store(h.0 as isize, Ordering::SeqCst);
    log::info!("PopResult: window attached");
    Ok(())
}

fn apply_styles(h: HWND) {
    // SAFETY: 读写活着的窗口的样式位；h 失效时调用失败。
    // 扩展样式：加 TOOLWINDOW | TOPMOST，去 APPWINDOW；不加 WS_EX_NOACTIVATE（浮窗需要拿焦点）。
    unsafe {
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
        let ex = (ex | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0) & !WS_EX_APPWINDOW.0;
        SetWindowLongPtrW(h, GWL_EXSTYLE, ex as isize);
        let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
        let unwanted =
            WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0 | WS_THICKFRAME.0;
        SetWindowLongPtrW(h, GWL_STYLE, ((style & !unwanted) | WS_POPUP.0) as isize);
    }
}

fn styles_intact(h: HWND) -> bool {
    // SAFETY: 同 apply_styles。
    let ex = unsafe { GetWindowLongPtrW(h, GWL_EXSTYLE) } as u32;
    ex & WS_EX_TOOLWINDOW.0 != 0
}

fn cloak(h: HWND, on: bool) {
    let value = BOOL::from(on);
    // SAFETY: value 在调用期间有效，长度与类型一致。
    if let Err(e) = unsafe {
        DwmSetWindowAttribute(
            h,
            DWMWA_CLOAK,
            (&raw const value).cast::<c_void>(),
            size_of::<BOOL>() as u32,
        )
    } {
        log::warn!("PopResult: cloak({on}) failed: {e}");
    }
}

pub fn show_result_window(rect: Rect) {
    let Some(h) = result_hwnd() else {
        return;
    };
    if !styles_intact(h) {
        log::warn!("PopResult: window styles were reset, reapplying");
        apply_styles(h);
    }
    // 显示前记下当时的前台窗口（关闭角/Esc关闭且前台仍是浮窗时归还焦点）；如果当前前台就是浮窗自己则不覆盖
    // SAFETY: 无指针参数。
    let fg = unsafe { GetForegroundWindow() };
    if fg != h {
        PREV_FOREGROUND.store(fg.0 as isize, Ordering::SeqCst);
    }

    let w = rect.r - rect.l;
    let h_px = rect.b - rect.t;
    // SAFETY: h 是活着的有效窗口。
    // 跨 DPI 显示器先只移动、再带尺寸（platform-windows.md §1）
    unsafe {
        // ignore: 移动失败下一行照样摆
        let _ = SetWindowPos(
            h,
            HWND_TOPMOST,
            rect.l,
            rect.t,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE,
        );
        // ignore: 摆放尺寸失败不致命
        let _ = SetWindowPos(h, HWND_TOPMOST, rect.l, rect.t, w, h_px, SWP_NOACTIVATE);
    }
    cloak(h, false);
    if !super::force_foreground(h) {
        log::warn!("PopResult: SetForegroundWindow refused");
    }
}

pub fn move_result_window(rect: Rect) {
    let Some(h) = result_hwnd() else {
        return;
    };
    let w = rect.r - rect.l;
    let h_px = rect.b - rect.t;
    // SAFETY: 仅修改位置和尺寸，不激活。
    // ignore: 调整尺寸失败不影响后续
    let _ = unsafe { SetWindowPos(h, HWND_TOPMOST, rect.l, rect.t, w, h_px, SWP_NOACTIVATE) };
}

pub fn hide_result_window() {
    let Some(h) = result_hwnd() else {
        return;
    };
    cloak(h, true);
    // 如果当前前台还是浮窗，归还焦点给记录的窗口；失焦隐藏时焦点已在别处，不归还
    // SAFETY: 无指针参数。
    let fg = unsafe { GetForegroundWindow() };
    if fg == h {
        let prev = PREV_FOREGROUND.swap(0, Ordering::SeqCst);
        if prev != 0 {
            let prev_h = HWND(prev as *mut c_void);
            // SAFETY: 尝试把前台归还给之前记录的窗口
            // ignore: 目标窗口已销毁或拒绝前台不致命
            let _ = unsafe { SetForegroundWindow(prev_h) };
        }
    } else {
        PREV_FOREGROUND.store(0, Ordering::SeqCst);
    }
}

pub fn result_window_focused() -> Option<bool> {
    let raw = RESULT_WINDOW.load(Ordering::SeqCst);
    if raw == 0 {
        return None;
    }
    let h = HWND(raw as *mut c_void);
    // SAFETY: 无指针参数。
    let fg = unsafe { GetForegroundWindow() };
    Some(fg == h)
}
