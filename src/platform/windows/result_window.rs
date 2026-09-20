//! 结果浮窗的原生窗口管理（Win32）。
//! 启动时建好并在屏幕外 show 一次，之后显示/隐藏全部用 DWM cloak + SetWindowPos，
//! 永远不调 Slint 的 show()/hide()（platform-windows.md §1）。

use std::ffi::c_void;
use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{BOOL, HWND, POINT};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_BORDER_COLOR, DWMWA_CLOAK, DWMWA_TRANSITIONS_FORCEDISABLED,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GWL_STYLE, GetClassNameW, GetCursorPos, GetForegroundWindow, GetSystemMetrics,
    GetWindowLongPtrW, GetWindowThreadProcessId, HWND_TOPMOST, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE, SW_SHOWNOACTIVATE,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, ShowWindow, WS_CAPTION, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
};

use crate::error::Error;
use crate::platform::geometry::Rect;

/// `DWMWA_BORDER_COLOR` 的特殊值：不画边框（windows crate 没导出这个常量）。
const DWMWA_COLOR_NONE: u32 = 0xFFFF_FFFE;

/// 圆角半径，逻辑像素。与 `ui/pop_result.slint` 那圈内描边的 `border-radius` 一致。
const CORNER_RADIUS: f32 = 8.0;

static RESULT_WINDOW: AtomicIsize = AtomicIsize::new(0);
static PREV_FOREGROUND: AtomicIsize = AtomicIsize::new(0);
/// 截图遮罩用自己的一份：它和结果浮窗会先后显示（框选完紧接着弹浮窗），
/// 共用一个记录位会让浮窗关闭时把焦点还给已经隐藏的遮罩。
static OVERLAY_WINDOW: AtomicIsize = AtomicIsize::new(0);
static OVERLAY_PREV_FOREGROUND: AtomicIsize = AtomicIsize::new(0);

fn result_hwnd() -> Option<HWND> {
    let raw = RESULT_WINDOW.load(Ordering::SeqCst);
    (raw != 0).then_some(HWND(raw as *mut c_void))
}

fn overlay_hwnd() -> Option<HWND> {
    let raw = OVERLAY_WINDOW.load(Ordering::SeqCst);
    (raw != 0).then_some(HWND(raw as *mut c_void))
}

/// 光标此刻在桌面上的物理坐标。取不到就当在原点（截图浮窗按光标摆放时用）。
pub fn cursor_pos() -> (i32, i32) {
    let mut point = POINT::default();
    // SAFETY: 只写一个栈上的 POINT，无其他指针。
    match unsafe { GetCursorPos(&mut point) } {
        Ok(()) => (point.x, point.y),
        Err(e) => {
            log::warn!("Platform: GetCursorPos failed: {e}");
            (0, 0)
        }
    }
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
    // Win11 标准圆角（8px；小圆角 4px 看着像普通窗口，B1 手测）
    let pref = DWMWCP_ROUND;
    // SAFETY: h 是活着的窗口；pref 在调用期间有效，长度与类型一致。
    if let Err(e) = unsafe {
        DwmSetWindowAttribute(
            h,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const pref).cast::<c_void>(),
            size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32,
        )
    } {
        // Win10：这个属性是 Win11 22000 才有的。窗口就是直角，而且系统会沿着直角边缘
        // 画一圈 1px 强调色（用户在「颜色」里选的，颜色因机而异，不能按颜色去 hack）。
        // 退回 SetWindowRgn 把四个角裁掉，见 `super::round_region`。
        log::info!("PopResult: DWM rounding unavailable, falling back to window region: {e}");
        super::mark_dwm_rounding_unavailable();
    }
    // 关掉 DWM 那 1px 边框：它画在客户区外面，左上角的红三角盖不住，会露出一圈浅灰。
    // 边框改由 pop_result.slint 里的 1px 内描边画。
    let none = DWMWA_COLOR_NONE;
    // SAFETY: h 是活着的窗口；none 在调用期间有效，长度与类型一致。
    if let Err(e) = unsafe {
        DwmSetWindowAttribute(
            h,
            DWMWA_BORDER_COLOR,
            (&raw const none).cast::<c_void>(),
            size_of::<u32>() as u32,
        )
    } {
        log::debug!("PopResult: DWM border not disabled: {e}");
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

pub fn attach_overlay_window(window: &slint::Window) -> Result<(), Error> {
    let h = super::hwnd(window)?;
    apply_styles(h);
    disable_transitions(h);
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
    OVERLAY_WINDOW.store(h.0 as isize, Ordering::SeqCst);
    log::info!("Overlay: window attached");
    Ok(())
}

/// 返回 `false` = 原生窗口还没交给平台层（`attach_overlay_window` 还没成功），这次截图显示不了。
/// 调用方必须收摊（清 `SHOT`、放开「正在截图」的位子），否则那个位子再也放不开。
pub fn show_overlay_window(rect: Rect) -> bool {
    let Some(h) = overlay_hwnd() else {
        return false;
    };
    if !styles_intact(h) {
        log::warn!("Overlay: window styles were reset, reapplying");
        apply_styles(h);
    }
    // 显示前记下当时的前台窗口：取消截图时要还回去，选完截图时也要先还回去，
    // 否则随后弹出的结果浮窗会把遮罩当成"用户原来在用的程序"。
    // SAFETY: 无指针参数。
    let fg = unsafe { GetForegroundWindow() };
    if fg != h {
        OVERLAY_PREV_FOREGROUND.store(fg.0 as isize, Ordering::SeqCst);
    }
    // 摆到位再解除 cloak，不会在旧位置闪。跨 DPI 显示器先只移动、再带尺寸。
    // SAFETY: h 是活着的有效窗口。
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
        let _ = SetWindowPos(
            h,
            HWND_TOPMOST,
            rect.l,
            rect.t,
            rect.r - rect.l,
            rect.b - rect.t,
            SWP_NOACTIVATE,
        );
    }
    cloak(h, false);
    // 遮罩要焦点：Esc 取消靠键盘事件。
    if !super::force_foreground(h) {
        log::warn!("Overlay: SetForegroundWindow refused");
    }
    true
}

pub fn hide_overlay_window() {
    let Some(h) = overlay_hwnd() else {
        return;
    };
    // cloak 而不是 SW_HIDE：遮罩要瞬间消失，SW_HIDE 会播系统的关窗淡出。
    cloak(h, true);
    // SAFETY: h 是活着的窗口。
    // ignore: 挪不回去也没关系，窗口已经看不见了
    let _ = unsafe {
        SetWindowPos(
            h,
            HWND_TOPMOST,
            -32000,
            -32000,
            0,
            0,
            SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
    // 前台还是遮罩自己时把焦点还回去，否则键盘输入会进一个看不见的窗口。
    // SAFETY: 无指针参数。
    let fg = unsafe { GetForegroundWindow() };
    let prev = OVERLAY_PREV_FOREGROUND.swap(0, Ordering::SeqCst);
    if fg == h && prev != 0 {
        let prev_h = HWND(prev as *mut c_void);
        // SAFETY: 把前台还给之前记录的窗口。
        // ignore: 目标窗口已销毁或拒绝前台不致命
        let _ = unsafe { SetForegroundWindow(prev_h) };
    }
}

/// 关掉这个窗口的 DWM 过渡动画。遮罩要瞬间出现，不许从中心缩放着展开。
fn disable_transitions(h: HWND) {
    let on = BOOL(1);
    // SAFETY: h 来自活着的 Slint 窗口；on 在调用期间有效，长度与类型一致。
    let result = unsafe {
        DwmSetWindowAttribute(
            h,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            (&raw const on).cast::<c_void>(),
            size_of::<BOOL>() as u32,
        )
    };
    if let Err(e) = result {
        log::warn!("Overlay: disable window transitions failed: {e}");
    }
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
    // 尺寸定了才能裁区域，而且要赶在显示之前，否则 Win10 上会闪一帧直角。
    super::round_region(h, w, h_px, CORNER_RADIUS);
    // SAFETY: h 是活着的有效窗口。
    unsafe {
        // 上一次是 SW_HIDE 隐藏的（见 hide_result_window），要重新显示；启动那次窗口本来就可见，是空操作。
        // 位置已经摆好才显示，不会在旧位置闪。
        // ignore: 已经可见时返回 false，不是错误
        let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
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
    // 内容撑大后窗口变高了，区域不会自己跟着长，要按新尺寸重裁。
    super::round_region(h, w, h_px, CORNER_RADIUS);
}

pub fn hide_result_window() {
    let Some(h) = result_hwnd() else {
        return;
    };
    // **先判断再隐藏，顺序不能换。** `SW_HIDE` 掉前台窗口时，系统会立刻另挑一个前台窗口，
    // 而它优先挑**本进程/本线程**的其他顶层窗口 —— 也就是设置窗口。
    // 原来的写法是先 `ShowWindow(SW_HIDE)` 再问 `GetForegroundWindow() == h`：
    // 那时前台早就被系统换成设置窗口了，判断永远为假，于是走 else 分支把记录清掉、
    // 焦点一次都没归还过。用户看到的就是"关掉译文，设置页自己弹出来"。
    // SAFETY: 无指针参数。
    let was_foreground = unsafe { GetForegroundWindow() } == h;
    let prev = PREV_FOREGROUND.swap(0, Ordering::SeqCst);
    // 焦点先还回去，再隐藏：系统不需要另挑前台，设置窗口也就没有机会被顶上来。
    // 失焦隐藏时焦点已经在别处，不归还。
    if was_foreground && prev != 0 {
        let prev_h = HWND(prev as *mut c_void);
        // SAFETY: 尝试把前台归还给之前记录的窗口；此刻前台还是我们自己的窗口，调用是被允许的。
        // ignore: 目标窗口已销毁或拒绝前台不致命
        let _ = unsafe { SetForegroundWindow(prev_h) };
    }
    // 用 SW_HIDE 而不是 cloak：系统关闭窗口的缩放淡出动画只在 ShowWindow 上走，cloak 是瞬间消失（旧版就是 SW_HIDE，
    // 用户说重构后关得太生硬）。platform-windows.md §1 第 4 条的"再显示是透明的"只发生在内容没变时，
    // 结果浮窗每次显示都重置 rows，整窗都是脏的。
    // SAFETY: h 是活着的窗口。
    // ignore: 隐藏失败不致命
    let _ = unsafe { ShowWindow(h, SW_HIDE) };
}

pub fn result_window_focused() -> Option<bool> {
    let raw = RESULT_WINDOW.load(Ordering::SeqCst);
    if raw == 0 {
        return None;
    }
    let h = HWND(raw as *mut c_void);
    // SAFETY: 无指针参数。
    let fg = unsafe { GetForegroundWindow() };
    let focused = fg == h;
    if !focused {
        log_foreground_thief(fg);
    }
    Some(focused)
}

/// 丢焦点是结果浮窗自动隐藏的**唯一**判据（`logic::popup_state`），
/// 所以"它自己莫名其妙关了"这类问题只能从"焦点被谁抢走了"查起。
/// 只在真判定失焦时记一条，不会刷屏。
fn log_foreground_thief(fg: HWND) {
    if fg.0.is_null() {
        log::debug!("PopResult: lost focus, no foreground window at all");
        return;
    }
    let mut pid = 0u32;
    let mut buf = [0u16; 64];
    // SAFETY: fg 非空；窗口可能已经销毁，那样这两个调用只是返回 0，不是 UB。
    let class = unsafe {
        // ignore: 要的是 pid（出参），线程 id 用不上
        let _ = GetWindowThreadProcessId(fg, Some(&mut pid));
        let n = GetClassNameW(fg, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    };
    // 本进程抢走的话就是我们自己的 bug（浮标、设置窗口都在同一个进程里）。
    let whose = if pid == std::process::id() {
        " <- 本进程"
    } else {
        ""
    };
    log::debug!(
        "PopResult: lost focus to hwnd={:?} pid={pid}{whose} class={class:?}",
        fg.0
    );
}
