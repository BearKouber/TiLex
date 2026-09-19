//! macOS 划词取词平台层实现：
//! - CGEventTap 监听鼠标键盘发现选中手势（tap 线程）
//! - AXUIElement 跨进程读取选中文本（worker 线程）
//! - 浮标窗口定位与展示（主线程 / Slint 事件循环）

use std::ptr::NonNull;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{OnceLock, RwLock};
use std::time::Instant;

use objc2::MainThreadMarker;
use objc2_app_kit::{
    NSEvent, NSEventMask, NSPasteboard, NSRunningApplication, NSScreen, NSWorkspace,
    NSWorkspaceDidActivateApplicationNotification,
};
use objc2_foundation::NSNotification;

use super::window;
use crate::error::Error;
use crate::platform::geometry::{DISMISS_DIST, Rect, dismissal_limit_squared, place};
use crate::platform::selection_state::{
    Candidates, ClipboardCandidate, Display, Displayed, Gesture, Offer, PendingGesture, ReadContext,
};
use crate::platform::{AcceptFn, BeforeShowFn, EngagedFn, EngagedSelection, SettingsFn};

pub mod ax;
pub mod clip_watch;
pub mod force_copy;
pub mod pasteboard;
pub mod tap;

/// 浮标的逻辑边长，和 `ui/pop_button.slint` 的 18px 一致。物理边长按目标显示器的 backingScaleFactor 换算。
const BUTTON_LOGICAL: f64 = 18.0;
const DRAG_MIN: i32 = 6;

pub enum Ev {
    Select(RawSelect),
    Clip(ClipboardCandidate),
    Cancel(u64),
    Hide(u64),
    Engage(u64),
}

pub struct RawSelect {
    pub id: u64,
    pub window: isize,
    pub cg_x: f64,
    pub cg_y: f64,
    pub at_ms: u32,
    pub clipboard_sequence: u32,
}

static TX: OnceLock<Sender<Ev>> = OnceLock::new();
static START_TIME: OnceLock<Instant> = OnceLock::new();

/// 手势序号与显示归属门控。
pub static CURRENT_GESTURE: AtomicU64 = AtomicU64::new(1);
pub static VISIBLE_GESTURE: AtomicU64 = AtomicU64::new(0);

/// 浮标在 CGEvent 点坐标系下的中心、尺寸与消失半径（供 tap 线程低开销判定）。
pub static BTN_PT_X: AtomicI32 = AtomicI32::new(0);
pub static BTN_PT_Y: AtomicI32 = AtomicI32::new(0);
pub static BTN_PT_SIZE: AtomicI32 = AtomicI32::new(18);
pub static BTN_DISMISS_LIMIT_SQUARED: AtomicU64 = AtomicU64::new(DISMISS_DIST * DISMISS_DIST);

#[derive(Clone, Copy, Debug)]
pub struct CachedScreen {
    pub cg_origin_x: f64,
    pub cg_origin_y: f64,
    pub cg_width: f64,
    pub cg_height: f64,
    pub work_area: Rect,
    pub scale: f64,
}

static SCREEN_CACHE: RwLock<Vec<CachedScreen>> = RwLock::new(Vec::new());

// ---------------------------------------------------------------- 启动与入口

pub fn start_selection(
    settings: SettingsFn,
    accept: AcceptFn,
    engaged: EngagedFn,
    before_show: BeforeShowFn,
) -> Result<(), Error> {
    // 1. 权限前置核查：若无「辅助功能」授权则主动请求弹窗并报错返回，绝不静默假装在跑。
    if !ax::is_process_trusted(true) {
        return Err(Error::Platform(
            "未获得「辅助功能」权限。请在「系统设置 › 隐私与安全性 › 辅助功能」中允许 TiLex，勾选后请重启 TiLex。".into(),
        ));
    }

    let (tx, rx) = channel();
    TX.set(tx)
        .map_err(|_| Error::Platform("selection already started".into()))?;

    // 2. 启动 CGEventTap 监听线程（创建失败会直接返回 Err）
    tap::start_tap_thread()?;

    // 3. 启动剪贴板宽限窗口轮询线程（创建失败会直接返回 Err）
    clip_watch::start_clip_watch_thread()?;

    // 4. 注册前台切换观察者（主线程注册，失败仅记录 warn，不影响核心功能）
    install_foreground_observer();
    install_escape_monitor();

    // 5. 初始刷新屏幕几何快照：
    // 这里就在主线程上（调用方是 ui/pop_button.rs），同步刷一次，
    // 保证第一次划词就有真实的屏幕几何可用。
    if let Some(mtm) = MainThreadMarker::new() {
        update_screen_cache_on_main(mtm);
    }

    // 6. 启动取词 worker 线程
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

    Ok(())
}

fn install_foreground_observer() {
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let block = block2::RcBlock::new(|_notif: NonNull<NSNotification>| {
        if let Some(gesture) = clip_watch::current_pending_gesture() {
            let current_id = CURRENT_GESTURE.load(Ordering::SeqCst);
            let new_pid = foreground_pid();
            let at_ms = current_at_ms();
            if gesture.id == current_id && gesture.interrupted_by(new_pid, at_ms) {
                tap::cancel_current();
            }
        }
    });

    // SAFETY:
    // - NSWorkspaceDidActivateApplicationNotification 是 AppKit 导出的 extern static 常量。
    // - block 具有 'static 生命周期，不捕获局部非 static 引用。
    let notif_name = unsafe { NSWorkspaceDidActivateApplicationNotification };
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(notif_name), None, None, &block)
    };
    // 观察者注册一次就随进程活到底：drop 掉 token 会把观察者摘掉，前台作废就失效了。
    // ObjC 对象不是 Sync，放不进 static，所以在这里故意泄漏一次引用。
    std::mem::forget(token);
}

fn install_escape_monitor() {
    let block = block2::RcBlock::new(|event: NonNull<NSEvent>| {
        // SAFETY: event 为系统传入的非空 NSEvent。
        let event_ref = unsafe { event.as_ref() };
        // 仅关注 Esc 键（macOS virtual keycode 53），其余按键立刻返回，不读取、不记录任何按键内容（隐私）。
        if event_ref.keyCode() == 53 {
            tap::cancel_current();
        }
    });

    let token =
        NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &block);
    match token {
        Some(token) => {
            // 监听注册一次就随进程活到底：drop 掉 token 会把监听摘掉。
            // ObjC 对象不是 Sync，放不进 static，所以在这里故意泄漏一次引用。
            std::mem::forget(token);
        }
        None => {
            log::warn!("Selection: failed to install global escape monitor");
        }
    }
}

pub fn engage_selection() {
    let owner = VISIBLE_GESTURE.load(Ordering::SeqCst);
    if owner != 0 {
        send(Ev::Engage(owner));
    }
}

pub fn send(ev: Ev) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(ev); // ignore: worker 没了只会是进程正在退出
    }
}

// ---------------------------------------------------------------- 浮标窗口显示与隐藏

fn show_at(x: i32, y: i32, px: i32, owner: u64, dismiss_limit: u64, scale: f64) {
    let s = if scale <= 0.0 { 1.0 } else { scale };
    let cx_pt = ((f64::from(x) + f64::from(px) / 2.0) / s).round() as i32;
    let cy_pt = ((f64::from(y) + f64::from(px) / 2.0) / s).round() as i32;
    let size_pt = (f64::from(px) / s).round() as i32;
    let limit_pt = ((dismiss_limit as f64) / (s * s)).round() as u64;

    BTN_PT_SIZE.store(size_pt, Ordering::Relaxed);
    BTN_PT_X.store(cx_pt, Ordering::Relaxed);
    BTN_PT_Y.store(cy_pt, Ordering::Relaxed);
    BTN_DISMISS_LIMIT_SQUARED.store(limit_pt, Ordering::Relaxed);
    VISIBLE_GESTURE.store(owner, Ordering::SeqCst);

    let rect = Rect {
        l: x,
        t: y,
        r: x + px,
        b: y + px,
    };
    log::info!("Selection: show button at ({x}, {y}) size {px}");
    // ignore: 事件循环没了只会是进程正在退出，那时不出浮标正是想要的
    let _ = slint::invoke_from_event_loop(move || {
        if VISIBLE_GESTURE.load(Ordering::SeqCst) == owner {
            window::show_button_window(rect);
        }
    });
}

fn hide_native(owner: u64) {
    // ignore: 已被 tap 线程清掉或换了新主人都不用管
    let _ = VISIBLE_GESTURE.compare_exchange(owner, 0, Ordering::SeqCst, Ordering::SeqCst);
    // ignore: 事件循环没了只会是进程正在退出，窗口跟着一起没
    let _ = slint::invoke_from_event_loop(|| {
        window::hide_button_window();
    });
}

// ---------------------------------------------------------------- 纯函数与坐标换算

/// 将 CGEvent 坐标系中的点 (px, py) 转换为 TiLex 桌面物理像素坐标 (x, y)。
/// - px, py: CGEvent 坐标系下的点（原点在主屏左上角，y 轴向下，单位为点）
/// - scale: 屏幕缩放因子（backingScaleFactor）
pub fn cg_point_to_desktop(px: f64, py: f64, scale: f64) -> (i32, i32) {
    let s = if scale <= 0.0 { 1.0 } else { scale };
    ((px * s).round() as i32, (py * s).round() as i32)
}

/// 匹配逗号分隔的黑名单配置项：
/// 不区分大小写，支持应用名称匹配以及 bundle id 末段匹配（如 `com.tencent.xinWeChat` → `xinWeChat`）。
pub fn matches_blacklist(app_name: Option<&str>, bundle_id: Option<&str>, list: &str) -> bool {
    if list.trim().is_empty() {
        return false;
    }
    let app_name_lower = app_name.map(str::to_lowercase);
    let bundle_last_lower =
        bundle_id.and_then(|id| id.split('.').next_back().map(str::to_lowercase));

    for item in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let item_lower = item.to_lowercase();
        let stripped = item_lower.strip_suffix(".exe").unwrap_or(&item_lower);
        if let Some(ref name) = app_name_lower
            && (name == &item_lower || name == stripped)
        {
            return true;
        }
        if let Some(ref last) = bundle_last_lower
            && (last == &item_lower || last == stripped)
        {
            return true;
        }
    }
    false
}

fn blacklisted(list: &str) -> bool {
    if list.trim().is_empty() {
        return false;
    }
    let Some(app) = NSWorkspace::sharedWorkspace().frontmostApplication() else {
        return false;
    };
    let app_name = app.localizedName();
    let app_name_str = app_name.as_deref().map(|s| s.to_string());
    let bundle_id = app.bundleIdentifier();
    let bundle_id_str = bundle_id.as_deref().map(|s| s.to_string());
    matches_blacklist(app_name_str.as_deref(), bundle_id_str.as_deref(), list)
}

pub fn foreground_pid() -> isize {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|app| app.processIdentifier() as isize)
        .unwrap_or(0)
}

pub fn foreground_is_ours() -> bool {
    let current_pid = NSRunningApplication::currentApplication().processIdentifier();
    let front_pid = foreground_pid();
    front_pid != 0 && front_pid == current_pid as isize
}

fn start_instant() -> Instant {
    *START_TIME.get_or_init(Instant::now)
}

pub fn current_at_ms() -> u32 {
    start_instant().elapsed().as_millis() as u32
}

pub fn current_clipboard_sequence() -> u32 {
    // changeCount 截断后极小概率为 0，selection_state 会将其视为无效而拒绝；
    // 这是安全的取舍（偏向拒绝而非误接受），不要使用 .max(1) 修正。
    NSPasteboard::generalPasteboard().changeCount() as u32
}

// ---------------------------------------------------------------- 屏幕几何缓存

pub fn refresh_screen_cache() {
    // ignore: 刷新失败时沿用上一份快照；真的一份都没有时 screen_for_point 返回 None，不出浮标
    let _ = slint::invoke_from_event_loop(|| {
        if let Some(mtm) = MainThreadMarker::new() {
            update_screen_cache_on_main(mtm);
        }
    });
}

pub fn update_screen_cache_on_main(mtm: MainThreadMarker) {
    let screens = NSScreen::screens(mtm);
    let Some(main_screen) = screens.firstObject() else {
        return;
    };
    let main_height = main_screen.frame().size.height;
    let count = screens.count();
    let mut list = Vec::with_capacity(count);
    for i in 0..count {
        let s = screens.objectAtIndex(i);
        let scale = s.backingScaleFactor();
        let frame = s.frame();
        let vf = s.visibleFrame();
        let work_area = window::appkit_to_rect(
            (vf.origin.x, vf.origin.y, vf.size.width, vf.size.height),
            main_height,
            scale,
        );
        list.push(CachedScreen {
            cg_origin_x: frame.origin.x,
            cg_origin_y: main_height - frame.origin.y - frame.size.height,
            cg_width: frame.size.width,
            cg_height: frame.size.height,
            work_area,
            scale,
        });
    }
    if let Ok(mut w) = SCREEN_CACHE.write() {
        *w = list;
    }
}

pub fn screen_for_point(cg_x: f64, cg_y: f64) -> Option<(Rect, f64)> {
    if let Ok(cache) = SCREEN_CACHE.read() {
        for s in cache.iter() {
            if cg_x >= s.cg_origin_x
                && cg_x < s.cg_origin_x + s.cg_width
                && cg_y >= s.cg_origin_y
                && cg_y < s.cg_origin_y + s.cg_height
            {
                return Some((s.work_area, s.scale));
            }
        }
        if let Some(first) = cache.first() {
            return Some((first.work_area, first.scale));
        }
    }
    None
}

pub fn screen_for_desktop(x: i32, y: i32) -> Option<(Rect, f64)> {
    if let Ok(cache) = SCREEN_CACHE.read() {
        for s in cache.iter() {
            if x >= s.work_area.l && x <= s.work_area.r && y >= s.work_area.t && y <= s.work_area.b
            {
                return Some((s.work_area, s.scale));
            }
        }
        if let Some(first) = cache.first() {
            return Some((first.work_area, first.scale));
        }
    }
    None
}

// ---------------------------------------------------------------- 取词 Worker

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
        while let Ok(ev) = rx.recv() {
            match ev {
                Ev::Select(raw) => self.on_select(raw),
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

    fn on_select(&mut self, raw: RawSelect) {
        refresh_screen_cache();
        let Some((work, scale)) = screen_for_point(raw.cg_x, raw.cg_y) else {
            log::warn!(
                "Selection: no screen geometry available for point ({}, {})",
                raw.cg_x,
                raw.cg_y
            );
            return;
        };
        let (x, y) = cg_point_to_desktop(raw.cg_x, raw.cg_y, scale);
        let gesture = Gesture {
            id: raw.id,
            window: raw.window,
            x,
            y,
            at_ms: raw.at_ms,
            clipboard_sequence: raw.clipboard_sequence,
        };
        if !gesture.is_current(self.read_context()) {
            log::info!("Selection: gesture {} superseded", raw.id);
            return;
        }
        self.candidates.begin(gesture);
        // 登记待处理手势必须赶在 AX 读之前：AX 是跨进程调用，可能比目标程序自己复制还慢
        // （platform-windows.md §4「剪贴板兜底的两个顺序陷阱」第 1 条）。
        // 用 worker 这一份 gesture，保证和 clipboard_ready 比较时结构完全相等。
        clip_watch::notify_gesture(PendingGesture::new(gesture));
        let mut text = ax::read_selected_text();
        log::info!("Selection: ax read {} chars", text.trim().chars().count());
        if text.trim().is_empty() {
            if self.force_copy_allowed(gesture) {
                text = force_copy::copy_selection(|| gesture.is_current(self.read_context()));
                log::info!(
                    "Selection: force copy got {} chars",
                    text.trim().chars().count()
                );
            } else {
                log::info!("Selection: force copy not eligible");
            }
        }
        let context = self.read_context();
        let mut offered = None;
        let pending = self
            .candidates
            .complete_uia(gesture, text, context, |c| offered = Some(c));
        if offered.is_none() && pending.is_none() {
            log::info!("Selection: no candidate offered");
        }
        if let Some(candidate) = offered {
            self.offer(candidate, work, scale);
        }
        if let Some(clip) = pending {
            self.on_clip(clip);
        }
    }

    /// 先读开关：关着时不做任何别的查询。
    fn force_copy_allowed(&self, gesture: Gesture) -> bool {
        let enabled = (self.settings)().force_copy;
        if !enabled {
            return false;
        }
        let has_selected_text_attr = ax::has_selected_text_attribute();
        let clipboard_changed = current_clipboard_sequence() != gesture.clipboard_sequence;
        let modifiers_down = force_copy::modifiers_down();
        let excluded_app = {
            let app = NSWorkspace::sharedWorkspace().frontmostApplication();
            let app_name = app
                .as_ref()
                .and_then(|a| a.localizedName())
                .map(|s| s.to_string());
            let bundle_id = app
                .as_ref()
                .and_then(|a| a.bundleIdentifier())
                .map(|s| s.to_string());
            matches_blacklist(
                app_name.as_deref(),
                bundle_id.as_deref(),
                force_copy::NO_FORCE_COPY,
            )
        };
        force_copy::eligible(&force_copy::Inputs {
            enabled,
            has_selected_text_attr,
            clipboard_changed,
            modifiers_down,
            excluded_app,
        }) && gesture.is_current(self.read_context())
    }

    /// 程序自己复制了选区（终端、选中即复制）。没注入任何东西，也不用恢复——文字本来就在那。
    fn on_clip(&mut self, clip: ClipboardCandidate) {
        if !self.candidates.clipboard_ready(clip, self.read_context()) {
            return;
        }
        let text = pasteboard::read_text().unwrap_or_default();
        // 读剪贴板可能让出给源程序：读完再复核序号、前台、手势和设置。
        let context = self.read_context();
        let mut offered = None;
        self.candidates
            .complete_clipboard(clip, text, context, |c| offered = Some(c));
        if let Some(candidate) = offered {
            refresh_screen_cache();
            if let Some((work, scale)) = screen_for_desktop(clip.gesture.x, clip.gesture.y) {
                self.offer(candidate, work, scale);
            }
        }
    }

    fn armed(&self) -> bool {
        let settings = (self.settings)();
        settings.enabled && !foreground_is_ours() && !blacklisted(&settings.blacklist)
    }

    fn read_context(&self) -> ReadContext {
        let gesture_id = CURRENT_GESTURE.load(Ordering::SeqCst);
        let window = foreground_pid();
        let armed = self.armed();
        let clipboard_sequence = current_clipboard_sequence();
        ReadContext {
            gesture_id,
            window,
            clipboard_sequence,
            armed: armed
                && CURRENT_GESTURE.load(Ordering::SeqCst) == gesture_id
                && foreground_pid() == window,
        }
    }

    fn offer(&mut self, candidate: Offer, work: Rect, scale: f64) {
        if !candidate.is_current(self.read_context()) || candidate.text.chars().count() < 2 {
            return;
        }
        if !(self.accept)(&candidate.text) {
            log::info!("PopButton: selection filtered out");
            return;
        }
        let settings = (self.settings)();
        let gesture = candidate.gesture;
        let px = (BUTTON_LOGICAL * scale).round() as i32;
        let anchor = Rect::point(gesture.x, gesture.y);
        let (sx, sy) = settings.corner;
        let (x, y, _) = place(anchor, px, px, sx, sy, settings.gap, work);
        let (cx, cy) = (x + px / 2, y + px / 2);
        let dismiss_limit = dismissal_limit_squared(anchor, cx, cy);
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
        show_at(x, y, px, gesture.id, dismiss_limit, scale);
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
        if CURRENT_GESTURE
            .compare_exchange(owner, owner + 1, Ordering::SeqCst, Ordering::SeqCst)
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

// ---------------------------------------------------------------- 单元测试

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cg_point_to_desktop_scale_identity() {
        let (x, y) = cg_point_to_desktop(100.0, 200.0, 1.0);
        assert_eq!(x, 100);
        assert_eq!(y, 200);
    }

    #[test]
    fn test_cg_point_to_desktop_scale_double() {
        let (x, y) = cg_point_to_desktop(100.0, 200.0, 2.0);
        assert_eq!(x, 200);
        assert_eq!(y, 400);
    }

    #[test]
    fn test_cg_point_to_desktop_negative_coordinates() {
        let (x, y) = cg_point_to_desktop(-50.0, -80.0, 2.0);
        assert_eq!(x, -100);
        assert_eq!(y, -160);
    }

    #[test]
    fn test_matches_blacklist_case_insensitive() {
        assert!(matches_blacklist(Some("WeChat"), None, "wechat"));
        assert!(matches_blacklist(Some("wechat"), None, "WeChat"));
        assert!(matches_blacklist(Some("WeChat"), None, "WECHAT.EXE"));
    }

    #[test]
    fn test_matches_blacklist_bundle_id_last_segment() {
        assert!(matches_blacklist(
            Some("微信"),
            Some("com.tencent.xinWeChat"),
            "xinwechat"
        ));
        assert!(matches_blacklist(None, Some("com.apple.Safari"), "safari"));
    }

    #[test]
    fn test_matches_blacklist_empty_list_blocks_nothing() {
        assert!(!matches_blacklist(
            Some("WeChat"),
            Some("com.tencent.xinWeChat"),
            ""
        ));
        assert!(!matches_blacklist(
            Some("WeChat"),
            Some("com.tencent.xinWeChat"),
            "   "
        ));
    }

    #[test]
    fn test_matches_blacklist_whitespace_and_empty_items() {
        assert!(matches_blacklist(
            Some("Safari"),
            Some("com.apple.Safari"),
            "  ,  wechat  , safari ,  "
        ));
        assert!(!matches_blacklist(
            Some("Finder"),
            Some("com.apple.finder"),
            "  ,  wechat  , safari ,  "
        ));
    }
}
