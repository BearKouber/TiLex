//! 划词（从旧版 `pop_button.rs` 搬来，去掉 Tauri）。低级鼠标钩子发现可能的选中手势，
//! 取词 worker 用 UI Automation 读选中的文字，**读到了才**在光标旁边显示浮标——
//! 光有鼠标手势不是证据，读不到文字 = 没有划词（platform-windows.md 的红线）。
//! UIA 不碰剪贴板也不碰键盘。
//!
//! UIA 读不到时还有两次机会，按顺序：
//! 1. 增强选中识别（`force_copy`，默认关）：两条 UIA 链上都没有 TextPattern 时（自己画文字的程序，
//!    如微信聊天气泡）模拟一次 Ctrl+C 再恢复剪贴板。条件见 force_copy.rs。
//! 2. 有的程序（终端、开了"选中即复制"的）自己把选区放进剪贴板。我们监听剪贴板，
//!    只认紧跟在选中手势之后的那一次更新。
//!
//! 浮标窗口：Slint 只 `show()` 一次，之后显示/隐藏只用裸 Win32（DWM cloak + SetWindowPos），
//! 因为 winit 在 Slint show/hide 时会整个重写窗口样式（B1 实测，见 platform-windows.md）。

mod force_copy;
pub(crate) use force_copy::write_text;

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::sync::OnceLock;
use std::sync::atomic::Ordering::{Relaxed, SeqCst};
use std::sync::atomic::{AtomicI32, AtomicIsize, AtomicU64};
use std::sync::mpsc::{Receiver, Sender, channel};

use windows::Win32::Foundation::{
    BOOL, CloseHandle, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DWM_WINDOW_CORNER_PREFERENCE, DWMWA_CLOAK, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUNDSMALL,
    DwmSetWindowAttribute,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, GetClipboardSequenceNumber,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, HWINEVENTHOOK, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    SetWinEventHook, UIA_TextPatternId,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetDoubleClickTime, VK_ESCAPE};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, CreateWindowExW, DefWindowProcW, DispatchMessageW, EVENT_SYSTEM_FOREGROUND,
    GWL_EXSTYLE, GWL_STYLE, GetForegroundWindow, GetMessageTime, GetMessageW, GetWindowLongPtrW,
    GetWindowThreadProcessId, HC_ACTION, HWND_MESSAGE, HWND_TOPMOST, KBDLLHOOKSTRUCT,
    MA_NOACTIVATE, MSG, MSLLHOOKSTRUCT, RegisterClassW, SWP_FRAMECHANGED, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SetWindowLongPtrW, SetWindowPos, SetWindowsHookExW, TranslateMessage,
    WH_KEYBOARD_LL, WH_MOUSE_LL, WINDOW_EX_STYLE, WINDOW_STYLE, WINEVENT_OUTOFCONTEXT,
    WM_CLIPBOARDUPDATE, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MOUSEACTIVATE,
    WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_SYSKEYDOWN, WNDCLASSW,
    WS_CAPTION, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_MAXIMIZEBOX,
    WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME,
};
use windows::core::{PWSTR, w};

use crate::error::Error;
use crate::platform::geometry::{
    DISMISS_DIST, Rect, dismissal_limit_squared, distance_squared, place,
};
use crate::platform::selection_state::{
    CLIP_GRACE_MS, Candidates, ClipboardCandidate, Display, Displayed, Gesture, Offer,
    PendingGesture, ReadContext,
};
use crate::platform::{AcceptFn, BeforeShowFn, EngagedFn, EngagedSelection, SettingsFn};

/// 浮标的逻辑边长，和 `ui/pop_button.slint` 的 18px 一致。物理边长按目标显示器的 DPI 算。
const BUTTON_LOGICAL: f64 = 18.0;
const DRAG_MIN: i32 = 6;
const DOUBLE_CLICK_SLOP: i32 = 4;

enum Ev {
    Select(Gesture),
    Clip(ClipboardCandidate),
    Cancel(u64),
    Hide(u64),
    Engage(u64),
}

/// 浮标窗口的 HWND（`HWND` 不是 Send，按整数存）。0 = 还没交进来。
static BUTTON: AtomicIsize = AtomicIsize::new(0);
static TX: OnceLock<Sender<Ev>> = OnceLock::new();
/// 钩子里立刻作废，即使 worker 还卡在跨进程 UIA 调用里。
/// 显示归属单独一份：旧的 Hide 不能影响新手势出的浮标。
static CURRENT_GESTURE: AtomicU64 = AtomicU64::new(1);
static VISIBLE_GESTURE: AtomicU64 = AtomicU64::new(0);
static BTN_X: AtomicI32 = AtomicI32::new(0);
static BTN_Y: AtomicI32 = AtomicI32::new(0);
static BTN_DISMISS_LIMIT_SQUARED: AtomicU64 = AtomicU64::new(DISMISS_DIST * DISMISS_DIST);
static BTN_PX: AtomicI32 = AtomicI32::new(18);

/// 事件都在钩子线程上产生。用 Cell 快照不用拿会阻塞的锁，发给 worker 之后就不再变。
#[derive(Clone, Copy)]
struct Press {
    x: i32,
    y: i32,
    double: bool,
}

thread_local! {
    static PRESS: Cell<Option<Press>> = const { Cell::new(None) };
    static LAST_UP: Cell<Option<(i32, i32, u32)>> = const { Cell::new(None) };
    /// 松开鼠标时就登记，赶在目标程序处理它、自己复制之前。
    static PENDING_SELECTION: Cell<Option<PendingGesture>> = const { Cell::new(None) };
}

// ---------------------------------------------------------------- 启动

pub fn start_selection(
    settings: SettingsFn,
    accept: AcceptFn,
    engaged: EngagedFn,
    before_show: BeforeShowFn,
) -> Result<(), Error> {
    let (tx, rx) = channel();
    TX.set(tx)
        .map_err(|_| Error::Platform("selection already started".into()))?;
    let worker = Worker {
        settings,
        accept,
        engaged,
        before_show,
        candidates: Candidates::default(),
        display: Display::default(),
    };
    std::thread::Builder::new()
        .name("selection-worker".into())
        .spawn(move || worker.run(rx))?;
    std::thread::Builder::new()
        .name("selection-hooks".into())
        .spawn(hook_thread)?;
    Ok(())
}

pub fn attach_selection_button(window: &slint::Window) -> Result<(), Error> {
    let h = super::hwnd(window)?;
    apply_styles(h);
    // SAFETY: 在窗口所属的 UI 线程上挂；button_proc 签名正确，窗口销毁时系统自动摘。
    if !unsafe { SetWindowSubclass(h, Some(button_proc), 1, 0) }.as_bool() {
        log::warn!(
            "PopButton: subclass failed, clicking the button may activate the result window"
        );
    }
    // 小圆角贴近旧版 5px 的圆角。Win10 不支持，直角照用。
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
        log::debug!("PopButton: no rounded corners: {e}");
    }
    cloak(h, true);
    // SAFETY: 只刷新样式，不动位置、大小、激活。
    // ignore: 刷新失败时样式照样生效，只是边框缓存晚一点更新
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
    BUTTON.store(h.0 as isize, SeqCst);
    log::info!("PopButton: window attached");
    Ok(())
}

/// 点浮标时 WS_EX_NOACTIVATE 只保证不激活浮标自己；默认处理仍回 MA_ACTIVATE，系统就把本线程
/// 上次的活动窗口（隐藏着的结果浮窗）拉到前台，手势因前台变化被取消（B1 实测：点击触发隔一次失败一次）。
unsafe extern "system" fn button_proc(
    h: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_MOUSEACTIVATE {
        return LRESULT(MA_NOACTIVATE as isize);
    }
    // SAFETY: 其余消息原样交给 winit 的窗口过程。
    unsafe { DefSubclassProc(h, msg, w, l) }
}

pub fn engage_selection() {
    let owner = VISIBLE_GESTURE.load(SeqCst);
    if owner != 0 {
        send(Ev::Engage(owner));
    }
}

fn button_hwnd() -> Option<HWND> {
    let raw = BUTTON.load(SeqCst);
    (raw != 0).then_some(HWND(raw as *mut c_void))
}

// ---------------------------------------------------------------- 浮标窗口

/// 不抢焦点（UIA 读的是焦点元素，点浮标抢了焦点就读不到了）、不进任务栏和 Alt+Tab、置顶；
/// 去掉标题栏那组样式（窗口有 WS_CAPTION 时系统会套最小尺寸）。
fn apply_styles(h: HWND) {
    // SAFETY: 读写一个活着的窗口的样式位；h 失效时这些调用只是失败。
    unsafe {
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
        let ex =
            (ex | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0) & !WS_EX_APPWINDOW.0;
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
    ex & WS_EX_NOACTIVATE.0 != 0 && ex & WS_EX_TOOLWINDOW.0 != 0
}

/// 隐藏 = DWM cloak。不用 ShowWindow(SW_HIDE)：隐藏再显示后 Slint 不知道要重画，窗口是透明的（B1 实测）。
/// cloak 的窗口看不见、也不参与命中测试（点不到），内容照常合成，取消后立刻是最新画面。
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
        log::warn!("PopButton: cloak({on}) failed: {e}");
    }
}

/// 只由 worker 调。先发布中心和消失半径，再发布显示归属，最后才让窗口出现。
fn show_at(x: i32, y: i32, px: i32, owner: u64, dismiss_limit: u64) {
    let Some(h) = button_hwnd() else {
        return;
    };
    if !styles_intact(h) {
        // 按实测不会发生（浮标之后再没调过 Slint 的 show/hide）；真发生了就补上，别让浮标抢焦点。
        log::warn!("PopButton: window styles were reset, reapplying");
        apply_styles(h);
    }
    // SAFETY: h 是活着的窗口；不激活。
    unsafe {
        // 先只移动：换到 DPI 不同的显示器时 winit 按逻辑尺寸重算大小（先带尺寸的话它会按旧 DPI 再放大一次）。
        // 再带尺寸摆准。
        // ignore: 失败时下一行照样摆
        let _ = SetWindowPos(h, HWND_TOPMOST, x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
        // ignore: 失败时浮标留在旧位置，移开鼠标就收起
        let _ = SetWindowPos(h, HWND_TOPMOST, x, y, px, px, SWP_NOACTIVATE);
    }
    BTN_PX.store(px, Relaxed);
    BTN_X.store(x + px / 2, Relaxed);
    BTN_Y.store(y + px / 2, Relaxed);
    BTN_DISMISS_LIMIT_SQUARED.store(dismiss_limit, Relaxed);
    VISIBLE_GESTURE.store(owner, SeqCst);
    cloak(h, false);
}

fn hide_native(owner: u64) {
    // 只有 worker 显示/隐藏窗口。钩子那边可能已经清掉了这个门，但它不会显示别的浮标。
    // ignore: 已被钩子清掉或换了新主人都不用管
    let _ = VISIBLE_GESTURE.compare_exchange(owner, 0, SeqCst, SeqCst);
    if let Some(h) = button_hwnd() {
        cloak(h, true);
    }
}

/// (x, y) 所在显示器的工作区（不含任务栏），以及那块显示器上浮标的物理边长。
/// MONITOR_DEFAULTTONEAREST 总会给一个显示器；拿不到信息只是理论上的事，那时退回整个虚拟桌面。
fn monitor_at(x: i32, y: i32) -> (Rect, i32) {
    let (work, scale) = super::result_window::work_area(x, y);
    let px = (BUTTON_LOGICAL * f64::from(scale)).round() as i32;
    (work, px)
}

// ---------------------------------------------------------------- 钩子线程

// ponytail: 钩子装上后到进程退出都不摘；开关在 worker 里判断。真发现常驻钩子有代价再加 PostThreadMessageW 拆除。
fn hook_thread() {
    // SAFETY: 回调都是本文件里签名正确的 extern "system" 函数；消息循环在本线程。
    unsafe {
        if let Err(e) = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0) {
            log::error!("PopButton: mouse hook failed, selection disabled: {e}");
            return;
        }
        if let Err(e) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), None, 0) {
            log::error!("PopButton: Escape cancellation hook failed: {e}");
        }
        let foreground_hook = SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground_proc),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        if foreground_hook.0.is_null() {
            log::error!("PopButton: foreground cancellation hook failed");
        }
    }
    install_clipboard_listener();
    let mut msg = MSG::default();
    // SAFETY: msg 在循环期间有效。
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        // SAFETY: 同上。
        unsafe {
            let _ = TranslateMessage(&msg); // ignore: 返回值只表示有没有生成字符消息
            DispatchMessageW(&msg);
        }
    }
    log::warn!("PopButton: hook message loop ended");
}

/// 在已经有消息泵的钩子线程上建一个 message-only 窗口，不多开线程、不轮询。
/// 靠 WM_CLIPBOARDUPDATE 知道哪些程序自己复制了选区。
fn install_clipboard_listener() {
    let class = w!("TiLexPopClipboard");
    // SAFETY: 取本模块句柄；类名是常量；窗口过程签名正确。
    unsafe {
        let hinstance: HINSTANCE = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(clip_proc),
            lpszClassName: class,
            hInstance: hinstance,
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            log::error!("PopButton: RegisterClassW failed");
            return;
        }
        match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            None,
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            None,
            hinstance,
            None,
        ) {
            Ok(h) => {
                if let Err(e) = AddClipboardFormatListener(h) {
                    log::error!("PopButton: AddClipboardFormatListener failed: {e}");
                }
            }
            Err(e) => log::error!("PopButton: message-only window failed: {e}"),
        }
    }
}

unsafe extern "system" fn clip_proc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_CLIPBOARDUPDATE {
        // 归属和序号在这里就记下，不等 worker 读剪贴板时再取。
        // 消息时间和 MSLLHOOKSTRUCT.time 是同一个 tick 时钟，且都在本线程，可以直接相减。
        PENDING_SELECTION.with(|pending| {
            let Some(mut captured) = pending.get() else {
                return;
            };
            let gesture = captured.gesture;
            // SAFETY: 无指针参数。
            let at_ms = unsafe { GetMessageTime() } as u32;
            if gesture.id != CURRENT_GESTURE.load(SeqCst)
                || at_ms.wrapping_sub(gesture.at_ms) > CLIP_GRACE_MS
            {
                return;
            }
            let window = foreground();
            if window != gesture.window {
                cancel_current();
                return;
            }
            // SAFETY: 无参数。
            let sequence = unsafe { GetClipboardSequenceNumber() };
            let candidate = captured.capture_clipboard(window, at_ms, sequence);
            pending.set(Some(captured));
            if let Some(candidate) = candidate {
                send(Ev::Clip(candidate));
            }
        });
        return LRESULT(0);
    }
    // SAFETY: 原样转给默认窗口过程。
    unsafe { DefWindowProcW(h, msg, w, l) }
}

unsafe extern "system" fn foreground_proc(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    at_ms: u32,
) {
    PENDING_SELECTION.with(|pending| {
        if let Some(captured) = pending.get() {
            let gesture = captured.gesture;
            if gesture.id == CURRENT_GESTURE.load(SeqCst)
                && gesture.interrupted_by(hwnd.0 as isize, at_ms)
            {
                cancel_current();
            }
        }
    });
}

unsafe extern "system" fn keyboard_proc(code: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: HC_ACTION 时 lParam 指向系统给的 KBDLLHOOKSTRUCT。
    if code == HC_ACTION as i32
        && matches!(w.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN)
        && unsafe { (*(l.0 as *const KBDLLHOOKSTRUCT)).vkCode } == u32::from(VK_ESCAPE.0)
    {
        PRESS.with(|press| press.set(None));
        cancel_current();
    }
    // SAFETY: 原样传给下一个钩子（键盘钩子永远不拦按键）。
    unsafe { CallNextHookEx(None, code, w, l) }
}

/// 低级钩子有 300ms 超时，超时会被系统静默摘掉（R-8）。这里只许线程本地快照、原子读写、
/// 便宜的元数据和 channel 发送；不许 UIA、读文字、读配置、记日志、拿会阻塞的锁。
unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        // SAFETY: HC_ACTION 时 lParam 指向系统给的 MSLLHOOKSTRUCT。
        let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let (x, y, time) = (info.pt.x, info.pt.y, info.time);
        match wparam.0 as u32 {
            WM_LBUTTONDOWN => on_down(x, y, time),
            WM_LBUTTONUP => on_up(x, y, time),
            WM_MOUSEMOVE => {
                if VISIBLE_GESTURE.load(SeqCst) != 0 && farther_than_button(x, y) {
                    cancel_current();
                }
            }
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL | WM_RBUTTONDOWN | WM_MBUTTONDOWN => {
                PRESS.with(|press| press.set(None));
                cancel_current();
            }
            _ => {}
        }
    }
    // SAFETY: 原样传给下一个钩子。
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

fn on_down(x: i32, y: i32, time: u32) {
    if VISIBLE_GESTURE.load(SeqCst) != 0 && over_button(x, y) {
        // 点不抢焦点的浮标：不能取消它自己的文字，也不能当成一次新的选中开始。
        PRESS.with(|press| press.set(None));
        return;
    }
    cancel_current();
    // 低级钩子收不到 WM_LBUTTONDBLCLK，双击自己配对。
    // SAFETY: 无参数。
    let double_click_ms = unsafe { GetDoubleClickTime() };
    let double = LAST_UP.with(|last| {
        last.get().is_some_and(|(lx, ly, at)| {
            (x - lx).abs() <= DOUBLE_CLICK_SLOP
                && (y - ly).abs() <= DOUBLE_CLICK_SLOP
                && time.wrapping_sub(at) <= double_click_ms
        })
    });
    PRESS.with(|press| press.set(Some(Press { x, y, double })));
}

/// 拖动或双击只是**候选**。到底有没有选中，由 worker 问 UIA 要文字来决定。
fn on_up(x: i32, y: i32, time: u32) {
    let Some(press) = PRESS.with(Cell::take) else {
        return;
    };
    let dx = x - press.x;
    let dy = y - press.y;
    let dragged = dx * dx + dy * dy > DRAG_MIN * DRAG_MIN;
    LAST_UP.with(|last| last.set(Some((x, y, time))));
    if dragged || press.double {
        let gesture = Gesture {
            id: CURRENT_GESTURE.fetch_add(1, SeqCst) + 1,
            window: foreground(),
            x,
            y,
            at_ms: time,
            // SAFETY: 无参数。
            clipboard_sequence: unsafe { GetClipboardSequenceNumber() },
        };
        PENDING_SELECTION.with(|pending| pending.set(Some(PendingGesture::new(gesture))));
        send(Ev::Select(gesture));
    }
}

fn farther_than_button(x: i32, y: i32) -> bool {
    distance_squared(x, y, BTN_X.load(Relaxed), BTN_Y.load(Relaxed))
        > BTN_DISMISS_LIMIT_SQUARED.load(Relaxed)
}

fn over_button(x: i32, y: i32) -> bool {
    let size = BTN_PX.load(Relaxed);
    let left = BTN_X.load(Relaxed) - size / 2;
    let top = BTN_Y.load(Relaxed) - size / 2;
    x >= left && x < left + size && y >= top && y < top + size
}

fn cancel_current() {
    let owner = CURRENT_GESTURE.fetch_add(1, SeqCst);
    PENDING_SELECTION.with(|pending| pending.set(None));
    send(Ev::Cancel(owner));
    // 门在这里清：一次划过屏幕不会排上几百个 Hide。worker 仍按显示归属核对。
    let displayed = VISIBLE_GESTURE.swap(0, SeqCst);
    if displayed != 0 {
        send(Ev::Hide(displayed));
    }
}

fn send(ev: Ev) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(ev); // ignore: worker 没了只会是进程正在退出
    }
}

fn foreground() -> isize {
    // SAFETY: 无参数。
    unsafe { GetForegroundWindow() }.0 as isize
}

// ---------------------------------------------------------------- 取词 worker

struct Worker {
    settings: SettingsFn,
    accept: AcceptFn,
    engaged: EngagedFn,
    before_show: BeforeShowFn,
    candidates: Candidates,
    display: Display,
}

impl Worker {
    fn run(mut self, rx: Receiver<Ev>) {
        // UIA 与套间无关，这个线程没有消息泵，用 MTA。
        // SAFETY: 本线程初始化一次，进程结束前不反初始化。
        if let Err(e) = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok() {
            log::error!("PopButton: CoInitializeEx failed: {e}");
        }
        while let Ok(ev) = rx.recv() {
            match ev {
                Ev::Select(gesture) => self.on_select(gesture),
                Ev::Clip(clip) => self.on_clip(clip),
                Ev::Cancel(owner) => {
                    self.candidates.cancel(owner);
                    self.hide_owned(owner);
                }
                Ev::Hide(owner) => self.hide_owned(owner),
                Ev::Engage(owner) => self.engage(owner),
            }
        }
    }

    /// 顺序要紧：趁源程序还拿着焦点先读文字，然后才把窗口放到屏幕上。
    fn on_select(&mut self, gesture: Gesture) {
        if !gesture.is_current(self.read_context()) {
            return;
        }
        self.candidates.begin(gesture);
        let (mut text, saw_text_pattern) = uia_selected_text(gesture);
        if text.is_empty() && self.force_copy_allowed(gesture, saw_text_pattern) {
            text = force_copy::copy_selection(|| gesture.is_current(self.read_context()));
        }
        // 模拟复制的文字按 UIA 答案交付：恢复剪贴板已经改了序号，剪贴板那条路会拒掉它。
        let context = self.read_context();
        let mut offered = None;
        let pending = self
            .candidates
            .complete_uia(gesture, text, context, |c| offered = Some(c));
        if let Some(candidate) = offered {
            self.offer(candidate);
        }
        if let Some(clip) = pending {
            self.on_clip(clip);
        }
    }

    /// 先读开关：关着时不做任何别的查询。
    fn force_copy_allowed(&self, gesture: Gesture, saw_text_pattern: bool) -> bool {
        let enabled = (self.settings)().force_copy;
        enabled
            && force_copy::eligible(&force_copy::Inputs {
                enabled,
                saw_text_pattern,
                // SAFETY: 无参数。
                clipboard_changed: unsafe { GetClipboardSequenceNumber() }
                    != gesture.clipboard_sequence,
                modifiers_down: force_copy::modifiers_down(),
                excluded_app: foreground_process_name()
                    .is_some_and(|name| matches_blacklist(&name, force_copy::NO_FORCE_COPY)),
            })
            && gesture.is_current(self.read_context())
    }

    /// 程序自己复制了选区（终端、选中即复制）。没注入任何东西，也不用恢复——文字本来就在那。
    fn on_clip(&mut self, clip: ClipboardCandidate) {
        if !self.candidates.clipboard_ready(clip, self.read_context()) {
            return;
        }
        let text = force_copy::read_text().unwrap_or_default();
        // 读剪贴板可能让出给源程序：读完再复核序号、前台、手势和设置。
        let context = self.read_context();
        let mut offered = None;
        self.candidates
            .complete_clipboard(clip, text, context, |c| offered = Some(c));
        if let Some(candidate) = offered {
            self.offer(candidate);
        }
    }

    fn armed(&self) -> bool {
        let settings = (self.settings)();
        settings.enabled && !foreground_is_ours() && !blacklisted(&settings.blacklist)
    }

    fn read_context(&self) -> ReadContext {
        // 读设置、查进程要花时间：取完快照时再核对前台和手势还是同一个。
        let gesture_id = CURRENT_GESTURE.load(SeqCst);
        let window = foreground();
        let armed = self.armed();
        // SAFETY: 无参数。
        let clipboard_sequence = unsafe { GetClipboardSequenceNumber() };
        ReadContext {
            gesture_id,
            window,
            clipboard_sequence,
            armed: armed && CURRENT_GESTURE.load(SeqCst) == gesture_id && foreground() == window,
        }
    }

    fn offer(&mut self, candidate: Offer) {
        if !candidate.is_current(self.read_context()) || candidate.text.chars().count() < 2 {
            return;
        }
        if !(self.accept)(&candidate.text) {
            log::info!("PopButton: selection filtered out");
            return;
        }
        let settings = (self.settings)();
        let gesture = candidate.gesture;
        let (work, px) = monitor_at(gesture.x, gesture.y);
        let anchor = Rect::point(gesture.x, gesture.y);
        let (sx, sy) = settings.corner;
        let (x, y, _) = place(anchor, px, px, sx, sy, settings.gap, work);
        let (cx, cy) = (x + px / 2, y + px / 2);
        let dismiss_limit = dismissal_limit_squared(anchor, cx, cy);
        // 语种过滤和摆放也可能比手势活得久：到这里最后核对一次才动窗口。
        if !candidate.is_current(self.read_context()) {
            return;
        }
        log::info!(
            "PopButton: showing gesture {} for {} chars",
            gesture.id,
            candidate.text.chars().count()
        );
        let clipboard = candidate.clipboard;
        self.display.show(Displayed {
            gesture,
            text: candidate.text,
            x: cx,
            y: cy,
        });
        (self.before_show)();
        show_at(x, y, px, gesture.id, dismiss_limit);
        let context = self.read_context();
        if !gesture.is_current(context) || clipboard.is_some_and(|clip| !clip.is_current(context)) {
            self.hide_owned(gesture.id);
        }
    }

    fn hide_owned(&mut self, owner: u64) {
        if self.display.take(owner).is_some() {
            hide_native(owner);
        }
    }

    fn engage(&mut self, owner: u64) {
        let Some(shown) = self.display.take(owner) else {
            return;
        };
        hide_native(owner);
        self.candidates.cancel(owner);
        if !shown.gesture.is_current(self.read_context()) {
            return;
        }
        // 挡住还在路上的 UIA / 剪贴板结果和重复的悬停/点击，不作废更新的手势。
        if CURRENT_GESTURE
            .compare_exchange(owner, owner + 1, SeqCst, SeqCst)
            .is_err()
        {
            return;
        }
        (self.engaged)(EngagedSelection {
            text: shown.text,
            x: shown.x,
            y: shown.y,
        });
    }
}

// ---------------------------------------------------------------- 读选区（UIA）

thread_local! {
    static UIA: RefCell<Option<IUIAutomation>> = const { RefCell::new(None) };
}

/// 返回选中的文字，以及两条链上有没有任何元素有 TextPattern。
/// 只有"哪儿都没有 TextPattern"才允许模拟复制，所以拿不准的情况一律报 true。
/// `IUIAutomation` 既不是 Send 也不是 Sync：实例在哪个线程建就在哪个线程用。
fn uia_selected_text(gesture: Gesture) -> (String, bool) {
    UIA.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            // SAFETY: 本线程已 CoInitializeEx。
            match unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_ALL) } {
                Ok(auto) => *slot = Some(auto),
                Err(e) => log::error!("PopButton: could not create the UIAutomation instance: {e}"),
            }
        }
        match slot.as_ref() {
            Some(auto) => read_selection(auto, gesture),
            None => (String::new(), true),
        }
    })
}

fn read_selection(auto: &IUIAutomation, gesture: Gesture) -> (String, bool) {
    let still_current =
        || foreground() == gesture.window && CURRENT_GESTURE.load(SeqCst) == gesture.id;
    if !still_current() {
        return (String::new(), true);
    }
    let foreground_hwnd = HWND(gesture.window as *mut c_void);
    // SAFETY: COM 调用，参数都是值或本函数里活着的接口。
    let Ok(root) = (unsafe { auto.ElementFromHandle(foreground_hwnd) }) else {
        log::debug!("PopButton: UIA foreground element unavailable");
        return (String::new(), true);
    };
    // SAFETY: 同上。
    let Ok(walker) = (unsafe { auto.RawViewWalker() }) else {
        log::debug!("PopButton: UIA raw tree walker unavailable");
        return (String::new(), true);
    };
    // 取不到或不在前台窗口树里的链算"没有 TextPattern"。
    let mut saw_text_pattern = false;
    // 文字叶子不一定实现 TextPattern：选区常常归外层文档。Raw view 保留了被过滤掉的包装元素。
    // 手势位置优先，免得读到一个无关的、有焦点的输入框。
    let point = POINT {
        x: gesture.x,
        y: gesture.y,
    };
    // SAFETY: 同上。
    let chains = unsafe {
        [
            ("pointer", auto.ElementFromPoint(point).ok()),
            ("focus", auto.GetFocusedElement().ok()),
        ]
    };
    for (source, element) in chains {
        let Some(element) = element else { continue };
        let Some(path) = scoped_ancestors(
            element,
            // SAFETY: 同上。
            |element| unsafe { walker.GetParentElement(element) }.ok(),
            |element| {
                // SAFETY: 同上。
                unsafe { auto.CompareElements(element, &root) }.is_ok_and(|v| v.as_bool())
            },
        ) else {
            log::debug!(
                "PopButton: UIA {source} outside foreground tree or ancestry limit reached"
            );
            continue;
        };
        for (depth, element) in path.into_iter().enumerate() {
            if !still_current() {
                return (String::new(), true);
            }
            match selection_of(&element) {
                UiaRead::Text(text) => {
                    log::debug!(
                        "PopButton: UIA selection via {source} at ancestor depth {depth} ({} chars)",
                        text.chars().count()
                    );
                    return (text, true);
                }
                UiaRead::EmptySelection => saw_text_pattern = true,
                UiaRead::NoPattern => {}
            }
        }
    }
    log::debug!(
        "PopButton: UIA no selected text in foreground ancestor chains (TextPattern seen: {saw_text_pattern})"
    );
    (String::new(), saw_text_pattern)
}

/// 读选区之前先验证整条链：只比进程号会把同一程序的其他窗口也放进来。
/// 不扫子孙，也不越过前台窗口走到桌面。
fn scoped_ancestors<T>(
    start: T,
    mut parent: impl FnMut(&T) -> Option<T>,
    mut is_root: impl FnMut(&T) -> bool,
) -> Option<Vec<T>> {
    let mut path = vec![start];
    for _ in 0..32 {
        let element = path.last()?;
        if is_root(element) {
            return Some(path);
        }
        if path.len() == 32 {
            break;
        }
        path.push(parent(element)?);
    }
    None
}

/// 有 TextPattern 但什么都没选中，本身就是答案（"没选中"），和"根本报不了选区"的元素不一样。
enum UiaRead {
    Text(String),
    EmptySelection,
    NoPattern,
}

fn selection_of(element: &IUIAutomationElement) -> UiaRead {
    // SAFETY: COM 调用，element 活着。
    let Ok(pattern) =
        (unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) })
    else {
        return UiaRead::NoPattern;
    };
    match selected_text(&pattern) {
        Some(text) => UiaRead::Text(text),
        None => UiaRead::EmptySelection,
    }
}

fn selected_text(pattern: &IUIAutomationTextPattern) -> Option<String> {
    // SAFETY: COM 调用，pattern 活着；下标在 Length 以内。
    unsafe {
        let ranges = pattern.GetSelection().ok()?;
        let mut out = String::new();
        for i in 0..ranges.Length().ok()? {
            out.push_str(&ranges.GetElement(i).ok()?.GetText(-1).ok()?.to_string());
        }
        let out = out.trim();
        (!out.is_empty()).then(|| out.to_string())
    }
}

// ---------------------------------------------------------------- 过滤

fn blacklisted(list: &str) -> bool {
    if list.trim().is_empty() {
        return false;
    }
    foreground_process_name().is_some_and(|name| matches_blacklist(&name, list))
}

fn matches_blacklist(process: &str, list: &str) -> bool {
    let process = process.to_lowercase();
    list.split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .any(|s| process == s || process == format!("{s}.exe"))
}

/// 自己的窗口不算：结果浮窗会拿焦点，翻译译文（或刚从里面复制的文字）从来不是用户想要的。
fn foreground_is_ours() -> bool {
    // SAFETY: pid 在调用期间有效。
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid == GetCurrentProcessId()
    }
}

/// 前台窗口的进程文件名（如 `WeChat.exe`）。只用于名单匹配，所以 UTF-16 转换丢字无所谓。
fn foreground_process_name() -> Option<String> {
    // SAFETY: buf / len 在调用期间有效；进程句柄用完就关。
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.0.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 260];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .is_ok();
        let _ = CloseHandle(handle); // ignore: 关不掉只是泄漏一个句柄
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().map(str::to_string)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_ancestors_stop_at_foreground_window() {
        let path = scoped_ancestors(4u32, |n| n.checked_sub(1), |n| *n == 1);
        assert_eq!(path, Some(vec![4, 3, 2, 1]));
        assert_eq!(
            scoped_ancestors(1u32, |_| panic!("must stop at root"), |n| *n == 1),
            Some(vec![1])
        );
    }

    #[test]
    fn selection_ancestors_reject_other_windows_and_cycles() {
        assert_eq!(
            scoped_ancestors(4u32, |n| n.checked_sub(1), |n| *n == 9),
            None
        );
        let mut calls = 0;
        let path = scoped_ancestors(
            4,
            |n| {
                calls += 1;
                Some(*n)
            },
            |n| *n == 9,
        );
        assert!(path.is_none());
        assert_eq!(calls, 31);
    }

    #[test]
    fn blacklist_matches_basename_case_insensitively() {
        assert!(matches_blacklist("Notepad.exe", "notepad"));
        assert!(matches_blacklist("notepad.exe", "Notepad.exe"));
        assert!(matches_blacklist("code.exe", " foo , code , bar "));
        assert!(!matches_blacklist("code.exe", "notepad"));
        assert!(!matches_blacklist("code.exe", ""));
        // 不能按子串匹配
        assert!(!matches_blacklist("vscode.exe", "code"));
    }

    #[test]
    fn no_force_copy_list_uses_blacklist_matching() {
        let list = force_copy::NO_FORCE_COPY;
        for name in [
            "WindowsTerminal.exe",
            "mintty.exe",
            "wezterm-gui.exe",
            "pwsh.exe",
            "termius.exe",
            "explorer.exe",
        ] {
            assert!(matches_blacklist(name, list), "{name}");
        }
        assert!(!matches_blacklist("Weixin.exe", list));
        assert!(!matches_blacklist("chrome.exe", list));
    }
}
