//! CGEventTap 监听鼠标键盘发现选中手势（运行于专属 CFRunLoop 后台线程）。

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::mpsc;

use objc2_core_foundation::{CFMachPort, CFRetained, CFRunLoop, kCFRunLoopCommonModes};
use objc2_core_graphics::{
    CGEvent, CGEventField, CGEventMask, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
    CGEventTapProxy, CGEventType,
};

use super::{
    BTN_DISMISS_LIMIT_SQUARED, BTN_PT_SIZE, BTN_PT_X, BTN_PT_Y, CURRENT_GESTURE, DRAG_MIN, Ev,
    RawSelect, VISIBLE_GESTURE, current_at_ms, current_clipboard_sequence, foreground_pid, send,
};
use crate::error::Error;

static TAP_PORT: AtomicPtr<c_void> = AtomicPtr::new(core::ptr::null_mut());

#[derive(Clone, Copy)]
struct Press {
    x: f64,
    y: f64,
    double: bool,
}

thread_local! {
    static PRESS: Cell<Option<Press>> = const { Cell::new(None) };
}

/// 判定拖动位移是否达到最小阈值（纯函数，供单元测试）。
pub fn is_dragged(dx: f64, dy: f64) -> bool {
    dx * dx + dy * dy > (DRAG_MIN * DRAG_MIN) as f64
}

pub fn start_tap_thread() -> Result<(), Error> {
    let (init_tx, init_rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("selection-tap".into())
        .spawn(move || {
            // macOS 10.15 起，只要 CGEventTapCreate 的掩码包含键盘事件，就需要独立的
            // 「输入监控」(Input Monitoring) TCC 权限，和「辅助功能」是两个开关。
            // 没授权时 CGEventTapCreate 照样返回有效 port、不报错，但一个事件都收不到，
            // 鼠标事件也收不到。所以掩码里只留鼠标事件，键盘的 Esc 走 NSEvent 全局监听（只要辅助功能权限）。
            let events_of_interest: CGEventMask = (1u64 << CGEventType::LeftMouseDown.0)
                | (1u64 << CGEventType::LeftMouseUp.0)
                | (1u64 << CGEventType::MouseMoved.0)
                | (1u64 << CGEventType::ScrollWheel.0)
                | (1u64 << CGEventType::RightMouseDown.0)
                | (1u64 << CGEventType::OtherMouseDown.0);

            // SAFETY: 回调函数 tap_callback 为 extern "C-unwind" 且具有正确签名；
            // tap 为 ListenOnly 模式，不修改任何事件。
            let tap = unsafe {
                CGEvent::tap_create(
                    CGEventTapLocation::SessionEventTap,
                    CGEventTapPlacement::HeadInsertEventTap,
                    CGEventTapOptions::ListenOnly,
                    events_of_interest,
                    Some(tap_callback),
                    core::ptr::null_mut(),
                )
            };

            let Some(port) = tap else {
                // ignore: 接收端只等这一次，收不到只可能是 start_selection 已经放弃等待
                let _ = init_tx.send(Err(Error::Platform(
                    "创建鼠标键盘监听（CGEventTap）失败，请检查系统辅助功能权限并在设置中允许 TiLex。".into(),
                )));
                return;
            };

            CGEvent::tap_enable(&port, true);

            TAP_PORT.store(
                CFRetained::as_ptr(&port).as_ptr().cast(),
                Ordering::SeqCst,
            );

            let Some(source) = CFMachPort::new_run_loop_source(None, Some(&port), 0) else {
                // ignore: 同上，接收端不在了说明调用方已经不等了
                let _ = init_tx.send(Err(Error::Platform(
                    "CFMachPortCreateRunLoopSource failed".into(),
                )));
                return;
            };

            let Some(rl) = CFRunLoop::current() else {
                // ignore: 同上，接收端不在了说明调用方已经不等了
                let _ = init_tx.send(Err(Error::Platform("CFRunLoopGetCurrent failed".into())));
                return;
            };

            // SAFETY: kCFRunLoopCommonModes 是 CoreFoundation 在进程启动时就初始化好的常量字符串，读取始终有效。
            let mode = unsafe { kCFRunLoopCommonModes };
            rl.add_source(Some(&source), mode);
            log::info!("Selection: event tap running");
            // ignore: 同上；发不出去时 tap 照样跑，只是调用方已经不等结果了
            let _ = init_tx.send(Ok(()));

            CFRunLoop::run();
        })?;

    init_rx
        .recv()
        .map_err(|_| Error::Platform("selection-tap thread exited unexpectedly".into()))?
}

unsafe extern "C-unwind" fn tap_callback(
    _proxy: CGEventTapProxy,
    r#type: CGEventType,
    event: NonNull<CGEvent>,
    _user_info: *mut c_void,
) -> *mut CGEvent {
    match r#type {
        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput => {
            let port_ptr = TAP_PORT.load(Ordering::SeqCst);
            if let Some(non_null) = NonNull::new(port_ptr) {
                // SAFETY: TAP_PORT 由本 run loop 线程在启动时初始化，指向活着的 CFMachPort；
                // 本回调由同一 run loop 驱动，生命周期内该指针持续有效。
                let port = unsafe { non_null.cast::<CFMachPort>().as_ref() };
                CGEvent::tap_enable(port, true);
                log::warn!("Selection: CGEventTap disabled by system, re-enabled");
            }
        }
        CGEventType::LeftMouseDown => {
            // SAFETY: event 为系统传入的非空 CGEvent。
            let event_ref = unsafe { event.as_ref() };
            let pt = CGEvent::location(Some(event_ref));
            let click_count =
                CGEvent::integer_value_field(Some(event_ref), CGEventField::MouseEventClickState);
            on_down(pt.x, pt.y, click_count);
        }
        CGEventType::LeftMouseUp => {
            // SAFETY: event 为系统传入的非空 CGEvent。
            let event_ref = unsafe { event.as_ref() };
            let pt = CGEvent::location(Some(event_ref));
            on_up(pt.x, pt.y);
        }
        CGEventType::MouseMoved => {
            // SAFETY: event 为系统传入的非空 CGEvent。
            let event_ref = unsafe { event.as_ref() };
            let pt = CGEvent::location(Some(event_ref));
            on_move(pt.x, pt.y);
        }
        CGEventType::ScrollWheel | CGEventType::RightMouseDown | CGEventType::OtherMouseDown => {
            PRESS.with(|press| press.set(None));
            cancel_current();
        }
        _ => {}
    }

    event.as_ptr()
}

fn on_down(x: f64, y: f64, click_count: i64) {
    if VISIBLE_GESTURE.load(Ordering::SeqCst) != 0 && over_button(x, y) {
        // 点击不抢焦点的浮标：不取消它自己的文字，也不算作新选中的开始。
        PRESS.with(|press| press.set(None));
        return;
    }
    cancel_current();
    // CGEvent 自带点击次数，>= 2 即为双击（无需在 macOS 侧手动按时间配对）。
    let double = click_count >= 2;
    PRESS.with(|press| press.set(Some(Press { x, y, double })));
}

fn on_up(x: f64, y: f64) {
    let Some(press) = PRESS.with(Cell::take) else {
        return;
    };
    let dx = x - press.x;
    let dy = y - press.y;
    let dragged = is_dragged(dx, dy);
    if dragged || press.double {
        let id = CURRENT_GESTURE.fetch_add(1, Ordering::SeqCst) + 1;
        let window = foreground_pid();
        let at_ms = current_at_ms();
        let clipboard_sequence = current_clipboard_sequence();
        let pt_x = x.round() as i32;
        let pt_y = y.round() as i32;
        log::info!(
            "Selection: gesture {id} at ({pt_x}, {pt_y}), dragged={dragged} double={}",
            press.double
        );
        send(Ev::Select(RawSelect {
            id,
            window,
            cg_x: x,
            cg_y: y,
            at_ms,
            clipboard_sequence,
        }));
    }
}

fn on_move(x: f64, y: f64) {
    if VISIBLE_GESTURE.load(Ordering::SeqCst) != 0 && farther_than_button(x, y) {
        cancel_current();
    }
}

pub fn cancel_current() {
    let owner = CURRENT_GESTURE.fetch_add(1, Ordering::SeqCst);
    PRESS.with(|press| press.set(None));
    send(Ev::Cancel(owner));
    let displayed = VISIBLE_GESTURE.swap(0, Ordering::SeqCst);
    if displayed != 0 {
        send(Ev::Hide(displayed));
    }
}

fn farther_than_button(x: f64, y: f64) -> bool {
    let px = x.round() as i32;
    let py = y.round() as i32;
    crate::platform::geometry::distance_squared(
        px,
        py,
        BTN_PT_X.load(Ordering::Relaxed),
        BTN_PT_Y.load(Ordering::Relaxed),
    ) > BTN_DISMISS_LIMIT_SQUARED.load(Ordering::Relaxed)
}

fn over_button(x: f64, y: f64) -> bool {
    let px = x.round() as i32;
    let py = y.round() as i32;
    let size = BTN_PT_SIZE.load(Ordering::Relaxed);
    let left = BTN_PT_X.load(Ordering::Relaxed) - size / 2;
    let top = BTN_PT_Y.load(Ordering::Relaxed) - size / 2;
    px >= left && px < left + size && py >= top && py < top + size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_dragged() {
        assert!(!is_dragged(0.0, 0.0));
        assert!(!is_dragged(4.0, 4.0)); // 16 + 16 = 32 <= 36
        assert!(is_dragged(6.0, 1.0)); // 36 + 1 = 37 > 36
        assert!(is_dragged(0.0, 7.0)); // 49 > 36
    }
}
