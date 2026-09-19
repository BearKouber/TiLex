//! macOS 原生窗口管理（AppKit / objc2）。
//! 负责浮标、结果浮窗、截图遮罩三个窗口的 attach / 显示 / 隐藏 / 定位，
//! 以及桌面光标与屏幕工作区查询。

use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplication, NSEvent, NSFloatingWindowLevel, NSScreen, NSScreenSaverWindowLevel, NSView,
    NSWindow, NSWindowCollectionBehavior,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::error::Error;
use crate::platform::geometry::Rect;

static BUTTON_WINDOW: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static RESULT_WINDOW: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());
static OVERLAY_WINDOW: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

/// 将 AppKit 坐标系中的矩形 (ox, oy, w, h) 转换为 TiLex 物理像素 Rect。
/// - ox, oy: AppKit 点坐标系下的原点（左下角）
/// - w, h: AppKit 点尺寸
/// - main_height: 主屏高度（点，NSScreen.screens()[0].frame().size.height）
/// - scale: 屏幕缩放因子（backingScaleFactor）
///
/// ponytail: 全局点坐标统一乘一个 scale。单屏、以及多屏同缩放都对；
/// 多屏**混合缩放**（比如 Retina 主屏 + 1x 外接屏）时副屏上的位置会偏 ——
/// macOS 没有 Windows 那种统一的物理像素虚拟桌面，要做对得按屏分段换算。
/// 朋友手测第 12 项（副屏划词）如果偏了，再按屏分段。
pub fn appkit_to_rect(rect: (f64, f64, f64, f64), main_height: f64, scale: f64) -> Rect {
    let (ox, oy, w, h) = rect;
    let s = if scale <= 0.0 { 1.0 } else { scale };
    let l = (ox * s).round() as i32;
    let t = ((main_height - oy - h) * s).round() as i32;
    let r = ((ox + w) * s).round() as i32;
    let b = ((main_height - oy) * s).round() as i32;
    Rect { l, t, r, b }
}

/// 将 TiLex 物理像素 Rect 转换为 AppKit 坐标系中的矩形 (ox, oy, w, h)。
/// - rect: TiLex 物理像素矩形
/// - main_height: 主屏高度（点）
/// - scale: 屏幕缩放因子
pub fn rect_to_appkit(rect: Rect, main_height: f64, scale: f64) -> (f64, f64, f64, f64) {
    let s = if scale <= 0.0 { 1.0 } else { scale };
    let ox = rect.l as f64 / s;
    let oy = main_height - (rect.b as f64 / s);
    let w = (rect.r - rect.l) as f64 / s;
    let h = (rect.b - rect.t) as f64 / s;
    (ox, oy, w, h)
}

/// 将 AppKit 坐标系中的点 (px, py) 转换为 TiLex 桌面物理像素坐标 (x, y)。
pub fn appkit_point_to_desktop(px: f64, py: f64, main_height: f64, scale: f64) -> (i32, i32) {
    let s = if scale <= 0.0 { 1.0 } else { scale };
    let x = (px * s).round() as i32;
    let y = ((main_height - py) * s).round() as i32;
    (x, y)
}

fn get_nswindow(window: &slint::Window) -> Result<Retained<NSWindow>, Error> {
    let handle = window.window_handle();
    let raw = handle
        .window_handle()
        .map_err(|e| Error::Platform(format!("window handle: {e}")))?
        .as_raw();
    match raw {
        RawWindowHandle::AppKit(h) => {
            // SAFETY: raw-window-handle 保证 h.ns_view 是有效的 NSView 指针。
            let view = unsafe { h.ns_view.cast::<NSView>().as_ref() };
            view.window()
                .ok_or_else(|| Error::Platform("NSView has no NSWindow yet".into()))
        }
        _ => Err(Error::Platform("not an AppKit window".into())),
    }
}

fn store_window(storage: &AtomicPtr<c_void>, window: Retained<NSWindow>) {
    let raw = Retained::into_raw(window).cast::<c_void>();
    let old = storage.swap(raw, Ordering::SeqCst);
    if !old.is_null() {
        // SAFETY: old 由先前的 Retained::into_raw 生成，持有 +1 引用计数。
        drop(unsafe { Retained::from_raw(old.cast::<NSWindow>()) });
    }
}

fn load_window(storage: &AtomicPtr<c_void>, _mtm: MainThreadMarker) -> Option<&'static NSWindow> {
    let ptr = storage.load(Ordering::SeqCst);
    if ptr.is_null() {
        None
    } else {
        // SAFETY: ptr 非空且由 store_window 维持 +1 强引用；
        // 调用方持有 MainThreadMarker，仅在主线程访问。
        unsafe { (ptr.cast::<NSWindow>() as *const NSWindow).as_ref() }
    }
}

fn screen_scale_for_rect(
    rect: Rect,
    screens: &objc2_foundation::NSArray<NSScreen>,
    main_height: f64,
) -> f64 {
    let cx = rect.l + (rect.r - rect.l) / 2;
    let cy = rect.t + (rect.b - rect.t) / 2;
    let count = screens.count();
    for i in 0..count {
        let s = screens.objectAtIndex(i);
        let scale = s.backingScaleFactor();
        let frame = s.frame();
        let screen_rect = appkit_to_rect(
            (
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            ),
            main_height,
            scale,
        );
        if cx >= screen_rect.l && cx < screen_rect.r && cy >= screen_rect.t && cy < screen_rect.b {
            return scale;
        }
    }
    screens
        .firstObject()
        .map(|s| s.backingScaleFactor())
        .unwrap_or(1.0)
}

pub(crate) fn button_window(mtm: MainThreadMarker) -> Option<&'static NSWindow> {
    load_window(&BUTTON_WINDOW, mtm)
}

/// 把已经显示的窗口拉到最前面并激活（最小化的先还原）。
pub fn bring_to_front(window: &slint::Window) -> Result<(), Error> {
    let mtm =
        MainThreadMarker::new().ok_or_else(|| Error::Platform("not on main thread".into()))?;
    let ns_window = get_nswindow(window)?;
    if ns_window.isMiniaturized() {
        ns_window.deminiaturize(None);
    }
    ns_window.makeKeyAndOrderFront(None);
    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    Ok(())
}

/// 把浮标窗口交给平台层。之后它的显示、隐藏、位置只由平台层管，调用方再也不能调它的 show()/hide()。
pub fn attach_selection_button(window: &slint::Window) -> Result<(), Error> {
    // 只做主线程守卫：NSWindow 的这些方法必须在主线程调，但它们不吃 MainThreadMarker。
    let _mtm =
        MainThreadMarker::new().ok_or_else(|| Error::Platform("not on main thread".into()))?;
    let w = get_nswindow(window)?;
    w.setLevel(NSFloatingWindowLevel);
    w.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    w.orderOut(None);
    store_window(&BUTTON_WINDOW, w);
    log::info!("PopButton: window attached");
    Ok(())
}

/// 把浮标摆到 rect（物理像素）并显示，不激活、不抢焦点（UI 线程调）。
pub fn show_button_window(rect: Rect) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(w) = button_window(mtm) else {
        return;
    };

    let screens = NSScreen::screens(mtm);
    let Some(main_screen) = screens.firstObject() else {
        return;
    };
    let main_height = main_screen.frame().size.height;
    let scale = screen_scale_for_rect(rect, &screens, main_height);

    let (ox, oy, width, height) = rect_to_appkit(rect, main_height, scale);
    let ns_rect = NSRect::new(NSPoint::new(ox, oy), NSSize::new(width, height));

    w.setFrame_display(ns_rect, true);
    w.orderFrontRegardless();
}

/// 隐藏浮标（UI 线程调）。
pub fn hide_button_window() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(w) = button_window(mtm) else {
        return;
    };
    w.orderOut(None);
}

/// 把结果浮窗交给平台层（UI 线程调）。
pub fn attach_result_window(window: &slint::Window) -> Result<(), Error> {
    // 只做主线程守卫：NSWindow 的这些方法必须在主线程调，但它们不吃 MainThreadMarker。
    let _mtm =
        MainThreadMarker::new().ok_or_else(|| Error::Platform("not on main thread".into()))?;
    let w = get_nswindow(window)?;
    w.setLevel(NSFloatingWindowLevel);
    w.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    w.orderOut(None);
    store_window(&RESULT_WINDOW, w);
    log::info!("PopResult: window attached");
    Ok(())
}

/// 在指定的屏幕矩形位置（物理像素）显示结果浮窗（UI 线程调）。
pub fn show_result_window(rect: Rect) {
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("PopResult: show called outside main thread");
        return;
    };
    let Some(w) = load_window(&RESULT_WINDOW, mtm) else {
        return;
    };

    let screens = NSScreen::screens(mtm);
    let Some(main_screen) = screens.firstObject() else {
        return;
    };
    let main_height = main_screen.frame().size.height;
    let scale = screen_scale_for_rect(rect, &screens, main_height);

    let (ox, oy, width, height) = rect_to_appkit(rect, main_height, scale);
    let ns_rect = NSRect::new(NSPoint::new(ox, oy), NSSize::new(width, height));

    w.setFrame_display(ns_rect, true);
    w.makeKeyAndOrderFront(None);

    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}

/// 调整结果浮窗在屏幕上的物理像素矩形位置与尺寸（UI 线程调）。
pub fn move_result_window(rect: Rect) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(w) = load_window(&RESULT_WINDOW, mtm) else {
        return;
    };

    let screens = NSScreen::screens(mtm);
    let Some(main_screen) = screens.firstObject() else {
        return;
    };
    let main_height = main_screen.frame().size.height;
    let scale = screen_scale_for_rect(rect, &screens, main_height);

    let (ox, oy, width, height) = rect_to_appkit(rect, main_height, scale);
    let ns_rect = NSRect::new(NSPoint::new(ox, oy), NSSize::new(width, height));

    w.setFrame_display(ns_rect, true);
}

/// 隐藏结果浮窗（UI 线程调）。
pub fn hide_result_window() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(w) = load_window(&RESULT_WINDOW, mtm) else {
        return;
    };
    // macOS 上应用为 LSUIElement，orderOut: 后系统自动将 key 状态归还给上一个应用。
    // 按任务规范先不记录、不归还前台焦点（见 02-window.md 要点 4）。
    w.orderOut(None);
}

/// 查询结果浮窗当前是否拥有系统前台焦点（UI 线程或计时器调）。
pub fn result_window_focused() -> Option<bool> {
    // 拿不到（没 attach、或不在主线程）一律 None＝「查不到」，不是 Some(false)。
    // 报成失焦会让调用方把浮窗关掉。
    let mtm = MainThreadMarker::new()?;
    let w = load_window(&RESULT_WINDOW, mtm)?;
    let app = NSApplication::sharedApplication(mtm);
    Some(app.isActive() && w.isKeyWindow())
}

/// 把截图遮罩交给平台层（UI 线程调，启动时一次）。
pub fn attach_overlay_window(window: &slint::Window) -> Result<(), Error> {
    // 只做主线程守卫：NSWindow 的这些方法必须在主线程调，但它们不吃 MainThreadMarker。
    let _mtm =
        MainThreadMarker::new().ok_or_else(|| Error::Platform("not on main thread".into()))?;
    let w = get_nswindow(window)?;
    // 遮罩要盖住 Dock 和菜单栏，层级设为 NSScreenSaverWindowLevel
    w.setLevel(NSScreenSaverWindowLevel);
    w.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    w.orderOut(None);
    store_window(&OVERLAY_WINDOW, w);
    log::info!("Overlay: window attached");
    Ok(())
}

/// 把遮罩摆到 rect（物理像素，整个虚拟屏）并显示、抢焦点（UI 线程调）。
/// 返回 false = 还没 attach，这次显示不了。
pub fn show_overlay_window(rect: Rect) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(w) = load_window(&OVERLAY_WINDOW, mtm) else {
        return false;
    };

    let screens = NSScreen::screens(mtm);
    let Some(main_screen) = screens.firstObject() else {
        return false;
    };
    let main_height = main_screen.frame().size.height;
    let scale = screen_scale_for_rect(rect, &screens, main_height);

    let (ox, oy, width, height) = rect_to_appkit(rect, main_height, scale);
    let ns_rect = NSRect::new(NSPoint::new(ox, oy), NSSize::new(width, height));

    w.setFrame_display(ns_rect, true);
    w.makeKeyAndOrderFront(None);

    let app = NSApplication::sharedApplication(mtm);
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);

    true
}

/// 隐藏遮罩（UI 线程调）。
pub fn hide_overlay_window() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(w) = load_window(&OVERLAY_WINDOW, mtm) else {
        return;
    };
    // macOS 上应用为 LSUIElement，orderOut: 后系统自动将 key 状态归还给上一个应用。
    // 按任务规范先不记录、不归还前台焦点（见 02-window.md 要点 4）。
    w.orderOut(None);
}

/// 光标此刻在桌面上的物理坐标。
pub fn cursor_pos() -> (i32, i32) {
    let pt = NSEvent::mouseLocation();
    let Some(mtm) = MainThreadMarker::new() else {
        log::warn!("Platform: cursor_pos not on main thread");
        return (0, 0);
    };
    let screens = NSScreen::screens(mtm);
    let Some(main_screen) = screens.firstObject() else {
        log::warn!("Platform: cursor_pos no screens available");
        return (0, 0);
    };
    let main_height = main_screen.frame().size.height;

    let mut target_scale = main_screen.backingScaleFactor();
    let count = screens.count();
    for i in 0..count {
        let s = screens.objectAtIndex(i);
        let f = s.frame();
        if pt.x >= f.origin.x
            && pt.x < f.origin.x + f.size.width
            && pt.y >= f.origin.y
            && pt.y < f.origin.y + f.size.height
        {
            target_scale = s.backingScaleFactor();
            break;
        }
    }

    appkit_point_to_desktop(pt.x, pt.y, main_height, target_scale)
}

/// (x, y) 所在显示器的工作区（物理像素，可为负坐标）和缩放比（DPI / 96）。
pub fn monitor_at(x: i32, y: i32) -> Option<(Rect, f32)> {
    let mtm = MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm);
    let main_screen = screens.firstObject()?;
    let main_height = main_screen.frame().size.height;

    let count = screens.count();
    for i in 0..count {
        let s = screens.objectAtIndex(i);
        let scale = s.backingScaleFactor();
        let frame = s.frame();
        let screen_rect = appkit_to_rect(
            (
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            ),
            main_height,
            scale,
        );
        if x >= screen_rect.l && x < screen_rect.r && y >= screen_rect.t && y < screen_rect.b {
            let vf = s.visibleFrame();
            let work_area = appkit_to_rect(
                (vf.origin.x, vf.origin.y, vf.size.width, vf.size.height),
                main_height,
                scale,
            );
            return Some((work_area, scale as f32));
        }
    }

    // 找不到包含该点的屏幕时，退回主屏工作区，保证不失败（要点 5）
    let scale = main_screen.backingScaleFactor();
    let vf = main_screen.visibleFrame();
    let work_area = appkit_to_rect(
        (vf.origin.x, vf.origin.y, vf.size.width, vf.size.height),
        main_height,
        scale,
    );
    Some((work_area, scale as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coordinate_roundtrip_main_screen() {
        let main_h = 1440.0;
        let scale = 2.0;

        let appkit_orig = (100.0, 200.0, 300.0, 400.0);
        let rect = appkit_to_rect(appkit_orig, main_h, scale);

        // 期望值使用字面量，不抄公式
        assert_eq!(
            rect,
            Rect {
                l: 200,
                t: 1680,
                r: 800,
                b: 2480,
            }
        );

        let appkit_back = rect_to_appkit(rect, main_h, scale);
        assert_eq!(appkit_back, (100.0, 200.0, 300.0, 400.0));
    }

    #[test]
    fn test_coordinate_roundtrip_secondary_above_main() {
        let main_h = 1440.0;
        let scale = 2.0;

        // 副屏在主屏上方：oy = 1440，w = 1920, h = 1080
        let appkit_above = (0.0, 1440.0, 1920.0, 1080.0);
        let rect = appkit_to_rect(appkit_above, main_h, scale);

        // 期望值使用字面量（t 为负）
        assert_eq!(
            rect,
            Rect {
                l: 0,
                t: -2160,
                r: 3840,
                b: 0,
            }
        );

        let appkit_back = rect_to_appkit(rect, main_h, scale);
        assert_eq!(appkit_back, (0.0, 1440.0, 1920.0, 1080.0));
    }

    #[test]
    fn test_appkit_point_to_desktop() {
        let main_h = 1440.0;
        let scale = 2.0;

        let pt_bl = appkit_point_to_desktop(100.0, 200.0, main_h, scale);
        assert_eq!(pt_bl, (200, 2480));

        let pt_tl = appkit_point_to_desktop(100.0, 600.0, main_h, scale);
        assert_eq!(pt_tl, (200, 1680));
    }

    #[test]
    #[ignore = "requires real macOS display and AppKit runtime"]
    fn test_monitor_at_real_screen() {
        let (area, scale) = monitor_at(100, 100).expect("main screen always exists");
        // 只断言 is_some 等于没测：工作区必须是个正经矩形，缩放必须是正数。
        assert!(area.r > area.l && area.b > area.t, "{area:?}");
        assert!(scale > 0.0, "{scale}");
    }
}
