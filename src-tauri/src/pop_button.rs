// Selection PopButton: a low-level mouse hook spots a text selection, then a
// small borderless window pops up next to the cursor. Hover or click it to
// translate the selection in a small floating result panel.
//
// The text is read through UI Automation, which touches neither the clipboard
// nor the keyboard. Reading it is also what tells us a selection happened at
// all: a mouse gesture on its own is not evidence, so the button only appears
// once we are actually holding the selected text.
//
// When UIA reads nothing there is one more chance: some apps (terminals, and
// anything configured to copy on select) put the selection on the clipboard
// themselves. We listen for that instead of synthesising Ctrl+C, and only
// accept an update that lands right after a selection gesture. Apps in neither
// camp get no button - the global selection-translate hotkey still covers them,
// since that path is allowed to synthesise Ctrl+C.

// ------------------------------------------------------------------ placement
//
// 划词浮标、划词弹窗、截图弹窗三处摆放都走 place()。放在 imp 外面，单测不用起窗口。

/// 物理像素矩形。光标、图标中心这种「点」就是 l == r、t == b 的零尺寸矩形。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub l: i32,
    pub t: i32,
    pub r: i32,
    pub b: i32,
}

impl Rect {
    pub fn point(x: i32, y: i32) -> Rect {
        Rect { l: x, t: y, r: x, b: y }
    }
}

/// 面板在某个轴上相对基准落在哪。
/// x 轴：Before = 基准左侧外，After = 右侧外，Start = 左边齐平，End = 右边齐平。
/// y 轴同理：Before = 上方，After = 下方，Start = 顶齐平，End = 底齐平。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Side {
    Before,
    After,
    Start,
    End,
}

impl Side {
    fn mirror(self) -> Side {
        match self {
            Side::Before => Side::After,
            Side::After => Side::Before,
            Side::Start => Side::End,
            Side::End => Side::Start,
        }
    }
}

// 一个轴：按 side 摆；越出 [min, max] 就换镜像那一侧；还放不下就把原来那一侧
// 夹回范围内（比范围还大时贴 min）。返回起点和最终用的 side。
// 两边都放不下时夹的是用户选的那一侧、不是镜像那侧：选了「下方」的面板被推上来
// 贴底，和老版本「出底就上推」一样；夹镜像那侧会把它甩到屏幕顶上去。返回的 side
// 也是这一侧，所以 place() 报的「在上方」和面板实际往哪长对得上。
fn place_axis(lo: i32, hi: i32, size: i32, side: Side, gap: i32, min: i32, max: i32) -> (i32, Side) {
    let at = |s: Side| match s {
        Side::Before => lo - gap - size,
        Side::After => hi + gap,
        Side::Start => lo,
        Side::End => hi - size,
    };
    let fits = |p: i32| p >= min && p + size <= max;
    let p = at(side);
    if fits(p) {
        return (p, side);
    }
    let q = at(side.mirror());
    if fits(q) {
        return (q, side.mirror());
    }
    (p.min(max - size).max(min), side)
}

/// 把 w x h 的窗口按 (sx, sy) 摆在 anchor 旁边，放不下先翻转再贴 bounds。
/// 返回 (x, y, 是否在基准上方)。第三项取翻转之后的结果 —— PopResult 靠它决定
/// 内容变高时钉住底边还是顶边。
pub fn place(anchor: Rect, w: i32, h: i32, sx: Side, sy: Side, gap: i32, bounds: Rect) -> (i32, i32, bool) {
    let (x, _) = place_axis(anchor.l, anchor.r, w, sx, gap, bounds.l, bounds.r);
    let (y, sy) = place_axis(anchor.t, anchor.b, h, sy, gap, bounds.t, bounds.b);
    (x, y, sy == Side::Before)
}

#[cfg(test)]
mod place_tests {
    use super::{place, Rect, Side::*};

    const SCREEN: Rect = Rect { l: 0, t: 0, r: 1000, b: 800 };

    #[test]
    fn each_direction_when_it_fits() {
        let p = Rect::point(500, 400);
        assert_eq!(place(p, 100, 50, After, After, 0, SCREEN), (500, 400, false));
        assert_eq!(place(p, 100, 50, Before, After, 0, SCREEN), (400, 400, false));
        assert_eq!(place(p, 100, 50, After, Before, 0, SCREEN), (500, 350, true));
        assert_eq!(place(p, 100, 50, Before, Before, 0, SCREEN), (400, 350, true));
        // gap 两个轴都留
        assert_eq!(place(p, 10, 10, Before, After, 4, SCREEN), (486, 404, false));
    }

    #[test]
    fn overflow_flips_to_the_other_side() {
        // 贴右边：翻到左侧
        assert_eq!(place(Rect::point(950, 400), 100, 50, After, After, 0, SCREEN), (850, 400, false));
        // 贴底边：翻到上方，而且要报告「在上方」
        assert_eq!(place(Rect::point(500, 780), 100, 50, After, After, 0, SCREEN), (500, 730, true));
        // 贴顶边选了上方：翻到下方
        assert_eq!(place(Rect::point(500, 10), 100, 50, After, Before, 0, SCREEN), (500, 10, false));
    }

    #[test]
    fn box_right_top_flips_to_the_left_of_the_box() {
        let sel = Rect { l: 700, t: 100, r: 950, b: 300 };
        assert_eq!(place(sel, 100, 50, After, Start, 4, SCREEN), (596, 100, false));
    }

    #[test]
    fn start_overflow_becomes_end() {
        // 选区贴底，顶齐平放不下 → 底齐平
        let sel = Rect { l: 100, t: 700, r: 300, b: 790 };
        assert_eq!(place(sel, 100, 200, After, Start, 4, SCREEN), (304, 590, false));
    }

    #[test]
    fn clamps_when_neither_side_fits() {
        // 上下都放不下：原来那一侧夹回屏内
        assert_eq!(place(Rect::point(500, 400), 100, 600, After, After, 0, SCREEN), (500, 200, false));
        assert_eq!(place(Rect::point(500, 400), 100, 600, After, Before, 0, SCREEN), (500, 0, true));
        // 比屏幕还大：贴左上
        assert_eq!(place(Rect::point(500, 400), 2000, 900, After, After, 0, SCREEN), (0, 0, false));
    }

    #[test]
    fn respects_a_bounds_that_does_not_start_at_zero() {
        // 副屏在主屏左边、工作区不含任务栏
        let work = Rect { l: -1920, t: 0, r: 0, b: 1040 };
        assert_eq!(place(Rect::point(-50, 1030), 320, 100, After, After, 0, work), (-370, 930, true));
    }
}

#[cfg(all(target_os = "windows", target_pointer_width = "64"))]
mod imp {
    use super::{place, Rect, Side};
    use crate::config::get as config_get;
    use crate::APP;
    use log::{error, info, warn};
    use once_cell::sync::OnceCell;
    use std::cell::RefCell;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering::Relaxed};
    use std::sync::mpsc::{channel, Receiver, Sender};
    use std::sync::Mutex;
    use tauri::Manager;
    use windows::core::{w, PWSTR};
    use windows::Win32::Foundation::{CloseHandle, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM};
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    use windows::Win32::System::DataExchange::AddClipboardFormatListener;
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
        PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
        UIA_TextPatternId,
    };
    use windows::Win32::UI::HiDpi::GetDpiForWindow;
    use windows::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime;
    use windows::Win32::UI::WindowsAndMessaging::*;

    // Window is BTN_LOGICAL points square. tao asks for that, but Windows
    // enforces SM_CXMIN/SM_CYMIN (136x39 at 96dpi) on any window that still
    // carries WS_CAPTION - which tao's undecorated windows do - so the real
    // size has to be forced on afterwards. BTN_PX holds the physical result.
    const BTN_LOGICAL: f64 = 18.0;
    const CLICK_SLOP: i32 = 20;
    const DRAG_MIN: i32 = 6;
    const DISMISS_DIST: i32 = 60;
    // Space between the cursor and the button (and between a screenshot box and
    // the panel beside it). The button defaults to bottom-left of the cursor:
    // the arrow and I-beam glyphs both extend down and to the right, so that
    // corner is the one that stays clear.
    const GAP: i32 = 4;
    const DOUBLE_CLICK_SLOP: i32 = 4;
    // How long after a selection gesture a clipboard update still counts as
    // that selection rather than as a Ctrl+C the user typed.
    const CLIP_GRACE_MS: u32 = 600;
    // The panel resizes itself to its content once the translation lands; this
    // is only the starting size, and what placement falls back to when the
    // real window size can't be read.
    const RESULT_LOGICAL_W: f64 = 320.0;
    const RESULT_LOGICAL_H: f64 = 100.0;

    enum Ev {
        Select(i32, i32),
        Clip(i32, i32),
        Hide,
    }

    static HWND_RAW: OnceCell<isize> = OnceCell::new();
    static TX: OnceCell<Sender<Ev>> = OnceCell::new();
    static PEEKED: Mutex<String> = Mutex::new(String::new());

    static VISIBLE: AtomicBool = AtomicBool::new(false);
    static BTN_X: AtomicI32 = AtomicI32::new(0);
    static BTN_Y: AtomicI32 = AtomicI32::new(0);
    // Physical size of the button, worked out from the window DPI at startup.
    static BTN_PX: AtomicI32 = AtomicI32::new(18);

    // Press tracking, written only from the hook callback.
    static DOWN_X: AtomicI32 = AtomicI32::new(0);
    static DOWN_Y: AtomicI32 = AtomicI32::new(0);
    static LAST_UP_MS: AtomicU32 = AtomicU32::new(0);
    static LAST_UP_X: AtomicI32 = AtomicI32::new(0);
    static LAST_UP_Y: AtomicI32 = AtomicI32::new(0);
    static IS_DOUBLE: AtomicBool = AtomicBool::new(false);
    // Timestamp of the last selection gesture, set by the hook rather than by
    // the worker: an app that copies on select can beat our UIA call to the
    // punch, so the clipboard update may arrive before we know UIA failed.
    // 0 means "no gesture is waiting for a clipboard update".
    static PENDING_CLIP_MS: AtomicU32 = AtomicU32::new(0);

    // ---------------------------------------------------------------- startup

    pub fn start() {
        let app = match APP.get() {
            Some(v) => v,
            None => return,
        };
        let window = match app.get_window("pop_button") {
            Some(v) => v,
            None => {
                match tauri::WindowBuilder::new(
                    app,
                    "pop_button",
                    tauri::WindowUrl::App("index.html".into()),
                )
                .inner_size(BTN_LOGICAL, BTN_LOGICAL)
                .position(0.0, 0.0)
                .visible(false)
                .decorations(false)
                .transparent(true)
                .always_on_top(true)
                .skip_taskbar(true)
                .focused(false)
                .resizable(false)
                .additional_browser_args("--disable-web-security")
                .build()
                {
                    Ok(v) => v,
                    Err(e) => {
                        error!("PopButton: create window failed: {}", e);
                        return;
                    }
                }
            }
        };

        // tauri returns a windows-0.39 HWND (a newtype over isize); we keep the
        // raw handle and rebuild our own. hwnd() round-trips to the event loop,
        // so it is called exactly once.
        let raw = match window.hwnd() {
            Ok(v) => v.0,
            Err(e) => {
                error!("PopButton: hwnd() failed: {}", e);
                return;
            }
        };
        let _ = HWND_RAW.set(raw);

        // WS_EX_NOACTIVATE keeps clicks from stealing focus, which would break
        // UIA (it reads the *focused* element). Applied once: from here on we
        // only ever show/hide through raw Win32, because tauri's show()/hide()
        // rewrites GWL_EXSTYLE and would silently drop these bits.
        unsafe {
            let h = HWND(raw as *mut c_void);
            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
            SetWindowLongPtrW(
                h,
                GWL_EXSTYLE,
                (ex | WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize,
            );

            // Drop the styles that make Windows apply a minimum size. Without
            // this the button is stuck at 136x39 no matter what tao asked for:
            // DefWindowProc's WM_GETMINMAXINFO clamps any window that still
            // has a caption to SM_CXMIN/SM_CYMIN.
            let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
            let unwanted = WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0;
            SetWindowLongPtrW(h, GWL_STYLE, ((style & !unwanted) | WS_POPUP.0) as isize);

            let px = (BTN_LOGICAL * GetDpiForWindow(h).max(96) as f64 / 96.0).round() as i32;
            BTN_PX.store(px, Relaxed);
            let _ = SetWindowPos(
                h,
                HWND_TOPMOST,
                0,
                0,
                px,
                px,
                SWP_NOMOVE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }

        build_result_window(app);

        let (tx, rx) = channel();
        let _ = TX.set(tx);
        std::thread::spawn(move || worker(rx));
        std::thread::spawn(hook_thread);
    }

    // Built up front and hidden, never closed: closing a window from the main
    // thread panics in tauri v1, and keeping the webview alive means the panel
    // opens instantly. Unlike the button this one is allowed to take focus, so
    // the translation can be selected and copied - which also means we never
    // touch GWL_EXSTYLE here and plain tauri calls are safe.
    fn build_result_window(app: &tauri::AppHandle) {
        if app.get_window("pop_result").is_some() {
            return;
        }
        match tauri::WindowBuilder::new(
            app,
            "pop_result",
            tauri::WindowUrl::App("index.html".into()),
        )
        .inner_size(RESULT_LOGICAL_W, RESULT_LOGICAL_H)
        .position(0.0, 0.0)
        .visible(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .additional_browser_args("--disable-web-security")
        .build()
        {
            // window.rs::build_window does this for every window it makes; the
            // pop windows bypass that path, so a borderless panel would sit on
            // the screen with no shadow at all.
            Ok(w) => {
                let _ = window_shadows::set_shadow(&w, true);
            }
            Err(e) => error!("PopButton: create result window failed: {}", e),
        }
    }

    fn button_hwnd() -> Option<HWND> {
        HWND_RAW.get().map(|v| HWND(*v as *mut c_void))
    }

    // ------------------------------------------------------------- mouse hook

    // ponytail: the hook stays installed until the process exits; the feature
    // toggle is checked in the worker instead. Add PostThreadMessageW teardown
    // if an always-on hook ever proves to cost something.
    fn hook_thread() {
        unsafe {
            let hook = match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0) {
                Ok(v) => v,
                Err(e) => {
                    error!("PopButton: SetWindowsHookExW failed: {}", e);
                    return;
                }
            };
            install_clipboard_listener();
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = UnhookWindowsHookEx(hook);
        }
    }

    // A message-only window on the thread that already runs a pump, so no
    // extra thread and no polling. WM_CLIPBOARDUPDATE is how we hear about
    // apps that copy their selection themselves.
    unsafe fn install_clipboard_listener() {
        let class = w!("TiLexPopClipboard");
        let hinstance: HINSTANCE = GetModuleHandleW(None).map(|m| m.into()).unwrap_or_default();
        let wc = WNDCLASSW {
            lpfnWndProc: Some(clip_proc),
            lpszClassName: class,
            hInstance: hinstance,
            ..Default::default()
        };
        if RegisterClassW(&wc) == 0 {
            error!("PopButton: RegisterClassW failed");
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
                    error!("PopButton: AddClipboardFormatListener failed: {}", e);
                }
            }
            Err(e) => error!("PopButton: message-only window failed: {}", e),
        }
    }

    unsafe extern "system" fn clip_proc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if msg == WM_CLIPBOARDUPDATE {
            // GetMessageTime and MSLLHOOKSTRUCT.time share the tick clock, and
            // this runs on the same thread as the hook, so the two are directly
            // comparable. Consuming the stamp keeps one gesture to one pop.
            let started = PENDING_CLIP_MS.swap(0, Relaxed);
            if started != 0 && (GetMessageTime() as u32).wrapping_sub(started) <= CLIP_GRACE_MS {
                send(Ev::Clip(LAST_UP_X.load(Relaxed), LAST_UP_Y.load(Relaxed)));
            }
            return LRESULT(0);
        }
        DefWindowProcW(h, msg, w, l)
    }

    // Low-level hooks are dropped by the OS if they take longer than
    // LowLevelHooksTimeout (300ms by default), so this only touches atomics and
    // occasionally pushes to the channel. Everything expensive is the worker's.
    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32 {
            let info = &*(lparam.0 as *const MSLLHOOKSTRUCT);
            let (x, y, time) = (info.pt.x, info.pt.y, info.time);
            match wparam.0 as u32 {
                WM_LBUTTONDOWN => on_down(x, y, time),
                WM_LBUTTONUP => on_up(x, y, time),
                WM_MOUSEMOVE => {
                    if VISIBLE.load(Relaxed) && farther_than(x, y, DISMISS_DIST) {
                        dismiss();
                    }
                }
                WM_MOUSEWHEEL => {
                    if VISIBLE.load(Relaxed) {
                        dismiss();
                    }
                }
                _ => {}
            }
        }
        CallNextHookEx(None, code, wparam, lparam)
    }

    fn on_down(x: i32, y: i32, time: u32) {
        if VISIBLE.load(Relaxed) && farther_than(x, y, CLICK_SLOP) {
            dismiss();
        }
        // A low-level hook never sees WM_LBUTTONDBLCLK, so pair up the clicks
        // ourselves.
        let elapsed = time.wrapping_sub(LAST_UP_MS.load(Relaxed));
        let near = (x - LAST_UP_X.load(Relaxed)).abs() <= DOUBLE_CLICK_SLOP
            && (y - LAST_UP_Y.load(Relaxed)).abs() <= DOUBLE_CLICK_SLOP;
        IS_DOUBLE.store(near && elapsed <= unsafe { GetDoubleClickTime() }, Relaxed);
        DOWN_X.store(x, Relaxed);
        DOWN_Y.store(y, Relaxed);
    }

    // A drag or a double click is only a *candidate*. Whether anything was
    // really selected is decided by the worker, which asks UIA for the text.
    fn on_up(x: i32, y: i32, time: u32) {
        let dx = x - DOWN_X.load(Relaxed);
        let dy = y - DOWN_Y.load(Relaxed);
        let dragged = dx * dx + dy * dy > DRAG_MIN * DRAG_MIN;
        LAST_UP_MS.store(time, Relaxed);
        LAST_UP_X.store(x, Relaxed);
        LAST_UP_Y.store(y, Relaxed);
        if dragged || IS_DOUBLE.swap(false, Relaxed) {
            PENDING_CLIP_MS.store(time, Relaxed);
            send(Ev::Select(x, y));
        }
    }

    fn farther_than(x: i32, y: i32, limit: i32) -> bool {
        let dx = x - BTN_X.load(Relaxed);
        let dy = y - BTN_Y.load(Relaxed);
        dx * dx + dy * dy > limit * limit
    }

    // Clearing VISIBLE here (not in the worker) stops one mouse sweep from
    // queueing hundreds of Hide events.
    fn dismiss() {
        VISIBLE.store(false, Relaxed);
        send(Ev::Hide);
    }

    fn send(ev: Ev) {
        if let Some(tx) = TX.get() {
            let _ = tx.send(ev);
        }
    }

    // ---------------------------------------------------------- worker thread

    fn worker(rx: Receiver<Ev>) {
        // UIA is apartment-agnostic and this thread has no message pump, so MTA.
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        while let Ok(ev) = rx.recv() {
            match ev {
                Ev::Select(x, y) => on_select(x, y),
                Ev::Clip(x, y) => on_clip(x, y),
                Ev::Hide => hide(),
            }
        }
    }

    // Order matters: read the text while the source app still owns the focus,
    // and only then put a window on screen.
    fn on_select(x: i32, y: i32) {
        if !armed() {
            return;
        }
        // No readable selection means this was a double click on a desktop
        // icon, a window drag, or an app UIA cannot see into. The first two get
        // nothing; the third may still turn up on the clipboard, so leave
        // PENDING_CLIP_MS alone and let clip_proc decide.
        let text = uia_selected_text(x, y);
        if text.is_empty() {
            return;
        }
        PENDING_CLIP_MS.store(0, Relaxed);
        offer(x, y, text);
    }

    // The app copied its own selection (a terminal, or anything set to copy on
    // select). Nothing was injected and nothing needs restoring - the text is
    // simply already there.
    fn on_clip(x: i32, y: i32) {
        if !armed() {
            return;
        }
        let text = clipboard_text();
        if !text.is_empty() {
            offer(x, y, text);
        }
    }

    fn armed() -> bool {
        config_bool("pop_button_enable", false) && !foreground_is_ours() && !blacklisted()
    }

    fn offer(x: i32, y: i32, text: String) {
        if text.chars().count() < 2 {
            return;
        }
        if config_bool("pop_button_exclude_native", true) && is_native_language(&text) {
            info!("PopButton: selection already in target language, ignoring");
            return;
        }

        info!("PopButton: showing for {} chars", text.chars().count());
        *PEEKED.lock().unwrap() = text;
        let px = BTN_PX.load(Relaxed);
        let (sx, sy) = corner(&config_string("pop_button_pos", "")).unwrap_or((Side::Before, Side::After));
        let (x, y, _) = place(Rect::point(x, y), px, px, sx, sy, GAP, work_area(x, y));
        show_at(x, y);
    }

    fn show_at(x: i32, y: i32) {
        let h = match button_hwnd() {
            Some(v) => v,
            None => return,
        };
        let px = BTN_PX.load(Relaxed);
        unsafe {
            // SetWindowPos both moves, sizes and shows, so tao's window flags
            // never change and the styles set at startup survive. The size is
            // repeated on every show because a DPI change would otherwise let
            // Windows put its own minimum back.
            let _ = SetWindowPos(
                h,
                HWND_TOPMOST,
                x,
                y,
                px,
                px,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            BTN_X.store(x + px / 2, Relaxed);
            BTN_Y.store(y + px / 2, Relaxed);
            VISIBLE.store(true, Relaxed);
        }
    }

    // 摆放的边界：(x, y) 所在显示器的工作区（不含任务栏），这样面板不会跨屏。
    // MONITOR_DEFAULTTONEAREST 总会给一个显示器，拿不到信息只是理论上的事，
    // 那时退回整个虚拟桌面。
    fn work_area(x: i32, y: i32) -> Rect {
        unsafe {
            let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(monitor, &mut info).as_bool() {
                let r = info.rcWork;
                return Rect { l: r.left, t: r.top, r: r.right, b: r.bottom };
            }
            let (l, t) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
            Rect {
                l,
                t,
                r: l + GetSystemMetrics(SM_CXVIRTUALSCREEN),
                b: t + GetSystemMetrics(SM_CYVIRTUALSCREEN),
            }
        }
    }

    // 「某个角对准一个点」的四个方位，配置值 → (x 轴, y 轴)。浮标、划词弹窗、
    // 截图弹窗的 cursor_* 共用。不认识的值返回 None，由调用方落到各自的默认值。
    fn corner(pos: &str) -> Option<(Side, Side)> {
        Some(match pos {
            "bottom_right" => (Side::After, Side::After),
            "bottom_left" => (Side::Before, Side::After),
            "top_right" => (Side::After, Side::Before),
            "top_left" => (Side::Before, Side::Before),
            _ => return None,
        })
    }

    fn hide() {
        if let Some(h) = button_hwnd() {
            unsafe {
                let _ = ShowWindow(h, SW_HIDE);
            }
        }
        VISIBLE.store(false, Relaxed);
    }

    // --------------------------------------------------------- engage (button)

    pub fn translate() {
        let (x, y) = (BTN_X.load(Relaxed), BTN_Y.load(Relaxed));
        hide();
        let text = std::mem::take(&mut *PEEKED.lock().unwrap());
        if text.is_empty() {
            warn!("PopButton: engaged with no cached selection");
            return;
        }
        let (sx, sy) = corner(&config_string("pop_result_pos", "")).unwrap_or((Side::After, Side::After));
        show_result(Rect::point(x, y), sx, sy, 0, text);
    }

    // 截图识别完由前端调进来。box_* 以选区为基准，cursor_* 以松手时的光标为基准。
    pub fn show_screenshot_result(sel: Rect, text: String) {
        let pos = config_string("screenshot_pos", "");
        let (anchor, sx, sy, gap) = match pos.as_str() {
            "box_right_top" => (sel, Side::After, Side::Start, GAP),
            // 本该是 x = 选区右边、y = 选区底 + GAP。place 两轴共用一个 gap，
            // 所以 x 也多出 GAP 个物理像素，看不出来。
            "box_bottom_right" => (sel, Side::After, Side::After, GAP),
            p => match p.strip_prefix("cursor_").and_then(corner) {
                Some((sx, sy)) => {
                    let mut pt = POINT::default();
                    let _ = unsafe { GetCursorPos(&mut pt) };
                    (Rect::point(pt.x, pt.y), sx, sy, 0)
                }
                // box_bottom_left，也是缺省和不认识的值：面板左上角对准选区左下角
                None => (sel, Side::Start, Side::After, 0),
            },
        };
        show_result(anchor, sx, sy, gap, text);
    }

    // Called from an async command, so the blocking getters below never run on
    // the main thread.
    fn show_result(anchor: Rect, sx: Side, sy: Side, gap: i32, text: String) {
        let window = match APP.get().and_then(|app| app.get_window("pop_result")) {
            Some(v) => v,
            None => return,
        };
        // 用面板此刻的真实大小（上一次内容撑出来的）来摆：内容高度没变时 JS 的
        // fit() 不会再动窗口，所以这里摆的位置必须和窗口现在的尺寸对得上。
        // ponytail: the size is in the DPI of whichever monitor the panel sat on
        // last, not of the one the cursor is on. Only off on the first pop after
        // moving to a screen with a different DPI.
        let (w, h) = match window.outer_size() {
            Ok(s) => (s.width as i32, s.height as i32),
            Err(_) => {
                let scale = window.scale_factor().unwrap_or(1.0);
                ((RESULT_LOGICAL_W * scale) as i32, (RESULT_LOGICAL_H * scale) as i32)
            }
        };
        let bounds = work_area((anchor.l + anchor.r) / 2, (anchor.t + anchor.b) / 2);
        // ponytail: 翻不翻只看此刻的高度。面板在下方、之后内容长到出屏，JS 那边
        // 只会贴边上推，可能压住划词处。要根治得按 MAX_HEIGHT 预判翻转，代价是
        // 屏幕下半区的面板会过早翻上去。
        let (x, y, above) = place(anchor, w, h, sx, sy, gap, bounds);
        let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
        let _ = window.show();
        let _ = window.set_focus();
        if let Some(h) = result_hwnd(&window) {
            unsafe { force_foreground(h) };
        }
        // 面板在基准上方时，内容变高要钉住底边往上长：把底边的物理 y 告诉 JS，
        // 下方时发 null。必须先于 new_text 发 —— 同一个窗口的事件按发送顺序到。
        let _ = window.emit("pop_anchor", above.then_some(y + h));
        let _ = window.emit("new_text", text);
    }

    // hwnd() 每次都要 round-trip 到事件循环，所以只问一次。
    fn result_hwnd(window: &tauri::Window) -> Option<HWND> {
        static RAW: OnceCell<isize> = OnceCell::new();
        let raw = *RAW.get_or_init(|| window.hwnd().map(|v| v.0).unwrap_or(0));
        (raw != 0).then(|| HWND(raw as *mut c_void))
    }

    // Windows 的前台锁：本进程不是前台的时候，SetForegroundWindow（tauri 的
    // set_focus 底下就是它）会被无声无息地拒掉 —— 窗口显示出来了，但没有焦点。
    //
    // 这条路上失败是常态，不是偶然：划词按钮是 WS_EX_NOACTIVATE 的，点它本来就
    // 不会让本进程变成前台。而**面板从来没拿到过焦点，就永远收不到 blur**，
    // 于是切到别处它也不消失 —— 前端那套 blur 逻辑再对也救不了。
    // （前台锁有几条例外条款，所以表现是「有时候不消失」而不是「从不消失」。）
    //
    // 把自己的输入队列临时挂到当前前台线程上，这条限制就不认了。挂完必须摘，
    // 一直挂着两个线程的输入状态就绑死了。
    // windows-0.58 没导出这个（元数据里就缺），自己声明。
    // BOOL AttachThreadInput(DWORD idAttach, DWORD idAttachTo, BOOL fAttach)
    #[link(name = "user32")]
    extern "system" {
        fn AttachThreadInput(id_attach: u32, id_attach_to: u32, attach: i32) -> i32;
    }

    unsafe fn force_foreground(hwnd: HWND) {
        let fg = GetForegroundWindow();
        let fg_thread = if fg.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        let me = GetCurrentThreadId();
        if fg_thread == 0 || fg_thread == me {
            let _ = SetForegroundWindow(hwnd);
            return;
        }
        AttachThreadInput(me, fg_thread, 1);
        let _ = SetForegroundWindow(hwnd);
        AttachThreadInput(me, fg_thread, 0);
    }

    // ----------------------------------------------------------- text reading

    thread_local! {
        static UIA: RefCell<Option<IUIAutomation>> = RefCell::new(None);
    }

    // IUIAutomation is neither Send nor Sync, so the instance lives and dies on
    // whichever thread built it.
    fn uia_selected_text(x: i32, y: i32) -> String {
        UIA.with(|cell| {
            let mut slot = cell.borrow_mut();
            if slot.is_none() {
                *slot = unsafe { CoCreateInstance(&CUIAutomation, None, CLSCTX_ALL) }.ok();
            }
            match slot.as_ref() {
                Some(auto) => unsafe { read_selection(auto, x, y) },
                None => {
                    error!("PopButton: could not create the UIAutomation instance");
                    String::new()
                }
            }
        })
    }

    unsafe fn read_selection(auto: &IUIAutomation, x: i32, y: i32) -> String {
        if let Some(text) = selection_of(auto.GetFocusedElement().ok()) {
            return text;
        }
        // Read-only text panels often never take the focus, so give whatever
        // sits under the cursor a turn. Still a real selection, just read from
        // a different element.
        selection_of(auto.ElementFromPoint(POINT { x, y }).ok()).unwrap_or_default()
    }

    unsafe fn selection_of(element: Option<IUIAutomationElement>) -> Option<String> {
        let pattern: IUIAutomationTextPattern =
            element?.GetCurrentPatternAs(UIA_TextPatternId).ok()?;
        let ranges = pattern.GetSelection().ok()?;
        let mut out = String::new();
        for i in 0..ranges.Length().ok()? {
            out.push_str(&ranges.GetElement(i).ok()?.GetText(-1).ok()?.to_string());
        }
        let out = out.trim().to_string();
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }

    fn clipboard_text() -> String {
        match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
            Ok(v) => v.trim().to_string(),
            Err(e) => {
                warn!("PopButton: clipboard read failed: {}", e);
                String::new()
            }
        }
    }

    // ---------------------------------------------------------------- filters

    fn config_bool(key: &str, default: bool) -> bool {
        config_get(key).and_then(|v| v.as_bool()).unwrap_or(default)
    }

    fn config_string(key: &str, default: &str) -> String {
        config_get(key)
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| default.to_string())
    }

    fn blacklisted() -> bool {
        let list = config_string("pop_button_blacklist", "");
        if list.trim().is_empty() {
            return false;
        }
        match foreground_process_name() {
            Some(name) => matches_blacklist(&name, &list),
            None => false,
        }
    }

    fn matches_blacklist(process: &str, list: &str) -> bool {
        let process = process.to_lowercase();
        list.split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .any(|s| process == s || process == format!("{}.exe", s))
    }

    // Our own windows are off limits: the result panel takes focus, and
    // translating the translation (or the text we just copied out of it) is
    // never what the user meant.
    fn foreground_is_ours() -> bool {
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

    fn foreground_process_name() -> Option<String> {
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
            let _ = CloseHandle(handle);
            if !ok {
                return None;
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            path.rsplit(['\\', '/']).next().map(|s| s.to_string())
        }
    }

    fn is_native_language(text: &str) -> bool {
        let target = config_string("translate_target_language", "zh_cn");
        // 两级：先用书写系统快速否掉（不同文字系统的一定不是目标语言，
        // 这一步不花钱），脚本对得上时才让 lingua 去区分同一书写系统里的
        // 语言 —— 英 / 法 / 德 这种脚本分不开的，只有真检测能分。
        if !is_native_language_for(text, &target) {
            return false;
        }
        // 本地识别没编进来（--no-default-features），或者文本太短认不出，
        // 就退回脚本那一层的结论。
        match crate::lang_detect::detect_code(text) {
            Some(code) => same_base_language(code, &target),
            None => true,
        }
    }

    // zh_cn / zh_tw、pt_pt / pt_br 这种只差地区的算同一种：lingua 本来也只
    // 返回大类（Language::Chinese → zh_cn），比到地区没有意义。
    fn same_base_language(a: &str, b: &str) -> bool {
        a.split('_').next() == b.split('_').next()
    }

    // Which writing system a character belongs to. Cheap enough to run on every
    // selection - lingua rebuilds its detector on every call (see
    // lang_detect.rs), which this path cannot afford.
    #[derive(PartialEq, Clone, Copy, Debug)]
    enum Script {
        Han,
        Kana,
        Hangul,
        Cyrillic,
        Arabic,
        Hebrew,
        Thai,
        Devanagari,
        Latin,
        Other,
    }

    fn script_of(c: char) -> Script {
        match c {
            '\u{4e00}'..='\u{9fff}' | '\u{3400}'..='\u{4dbf}' => Script::Han,
            '\u{3040}'..='\u{30ff}' => Script::Kana,
            '\u{1100}'..='\u{11ff}' | '\u{ac00}'..='\u{d7af}' => Script::Hangul,
            '\u{0400}'..='\u{04ff}' => Script::Cyrillic,
            '\u{0590}'..='\u{05ff}' => Script::Hebrew,
            '\u{0600}'..='\u{06ff}' => Script::Arabic,
            '\u{0e00}'..='\u{0e7f}' => Script::Thai,
            '\u{0900}'..='\u{097f}' => Script::Devanagari,
            // Everything else alphabetic is treated as Latin, accents included.
            c if c.is_alphabetic() => Script::Latin,
            _ => Script::Other,
        }
    }

    // ponytail: script matching, not language detection. Every Latin-script
    // language collapses into one bucket, so with an English target a French
    // selection also counts as "already the target" and gets no button. Real
    // detection here would mean a per-selection network call or lingua's
    // rebuild cost; swap one in only if that false skip actually bites.
    fn target_script(target: &str) -> Script {
        match target.split('_').next().unwrap_or(target) {
            "zh" => Script::Han,
            "ja" => Script::Kana,
            "ko" => Script::Hangul,
            "ru" | "uk" => Script::Cyrillic,
            "ar" | "fa" => Script::Arabic,
            "he" => Script::Hebrew,
            "th" => Script::Thai,
            "hi" => Script::Devanagari,
            _ => Script::Latin,
        }
    }

    fn is_native_language_for(text: &str, target: &str) -> bool {
        let want = target_script(target);
        let scripts: Vec<Script> = text
            .chars()
            .map(script_of)
            .filter(|s| *s != Script::Other)
            .collect();
        if scripts.is_empty() {
            return false;
        }
        let has_kana = scripts.iter().any(|s| *s == Script::Kana);
        match want {
            // Kana belongs to Japanese alone, so any of it settles the question
            // even when most of the text is kanji.
            Script::Kana => has_kana,
            // And the same fact read backwards: kana means it is not Chinese,
            // so it must not be skipped as "already the target".
            Script::Han if has_kana => false,
            _ => scripts.iter().filter(|s| **s == want).count() * 2 > scripts.len(),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::{is_native_language_for, matches_blacklist, same_base_language};

        #[test]
        fn native_language_skips_only_matching_script() {
            assert!(is_native_language_for("这是一段中文", "zh_cn"));
            assert!(is_native_language_for("中文,带标点!", "zh_cn"));
            assert!(!is_native_language_for("hello world", "zh_cn"));
            // mostly English with one Han char still deserves translating
            assert!(!is_native_language_for("hello world 中", "zh_cn"));
            // punctuation and digits alone must not count as native
            assert!(!is_native_language_for("12345 !!!", "zh_cn"));
        }

        // The whole point of dropping the secondary target language: whatever
        // the target is, text already in it must not get a button.
        #[test]
        fn native_language_covers_every_target_not_just_chinese() {
            assert!(is_native_language_for("hello world", "en"));
            assert!(is_native_language_for("bonjour le monde", "fr"));
            assert!(!is_native_language_for("这是一段中文", "en"));
            assert!(is_native_language_for("こんにちは", "ja"));
            assert!(is_native_language_for("안녕하세요", "ko"));
            assert!(is_native_language_for("привет мир", "ru"));
            assert!(!is_native_language_for("hello world", "ru"));
        }

        // Kana is the only thing separating Japanese from Chinese cheaply.
        #[test]
        fn kana_decides_between_japanese_and_chinese() {
            // kanji-heavy Japanese still has kana in it
            assert!(is_native_language_for("私は学生です", "ja"));
            // ...and that same kana must stop it counting as Chinese
            assert!(!is_native_language_for("私は学生です", "zh_cn"));
            // pure Han with a Japanese target is genuinely ambiguous; we do not
            // skip it, so the user still gets a button
            assert!(!is_native_language_for("東京", "ja"));
        }

        // Ceiling of the *script* layer alone: Latin-script languages are one
        // bucket. is_native_language() covers this by asking lingua after the
        // script check passes - this test pins the cheap layer, not the result.
        #[test]
        fn latin_targets_cannot_tell_latin_languages_apart() {
            assert!(is_native_language_for("bonjour le monde", "en"));
        }

        #[test]
        fn base_language_ignores_the_region_suffix() {
            assert!(same_base_language("zh_cn", "zh_tw"));
            assert!(same_base_language("pt_pt", "pt_br"));
            assert!(same_base_language("en", "en"));
            assert!(!same_base_language("en", "de"));
            assert!(!same_base_language("zh_cn", "ja"));
        }

        #[test]
        fn blacklist_matches_basename_case_insensitively() {
            assert!(matches_blacklist("Notepad.exe", "notepad"));
            assert!(matches_blacklist("notepad.exe", "Notepad.exe"));
            assert!(matches_blacklist("code.exe", " foo , code , bar "));
            assert!(!matches_blacklist("code.exe", "notepad"));
            assert!(!matches_blacklist("code.exe", ""));
            // must not match on a substring
            assert!(!matches_blacklist("vscode.exe", "code"));
        }
    }
}

#[cfg(not(all(target_os = "windows", target_pointer_width = "64")))]
mod imp {
    pub fn start() {}
    pub fn translate() {}
    pub fn show_screenshot_result(_sel: super::Rect, _text: String) {}
}

pub fn start_pop_button() {
    imp::start();
}

// Must be async so it never runs on the main thread: opening a window from
// here would otherwise contend with the event loop.
#[tauri::command(async)]
pub fn pop_button_translate() {
    imp::translate();
}

/// 把一段文字推进悬浮结果面板。截图识别完由前端调这个，
/// 四条边是 crop_region 返回的选区矩形（物理坐标），摆在哪由 `screenshot_pos` 定。
#[tauri::command(async)]
pub fn show_pop_result(left: i32, top: i32, right: i32, bottom: i32, text: String) {
    imp::show_screenshot_result(Rect { l: left, t: top, r: right, b: bottom }, text);
}
