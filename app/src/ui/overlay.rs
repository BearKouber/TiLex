//! 截图遮罩：全屏冻结画面 + 拖框选区。
//! 每次新建、用完销毁（design §1.5 窗口生命周期）。

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, PhysicalPosition, PhysicalSize};

use crate::error::Error;
use crate::platform::{self, Shot};
use crate::slint_ui::Overlay;

thread_local! {
    static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
}

/// 开一次截图。非 UI 线程调（抓屏几十毫秒，R-5）。
/// 内部：抓屏 → 切回 UI 线程建遮罩 → 用户框选 → 销毁遮罩 → 裁图。
pub fn start() {
    if !crate::logic::screenshot::begin() {
        log::info!("Screenshot: already in progress, ignored");
        return;
    }

    let trigger_time = Instant::now();
    std::thread::spawn(move || {
        let shot = match platform::capture_screen() {
            Ok(s) => s,
            Err(e) => {
                log::error!("Screenshot: capture_screen failed: {e}");
                crate::logic::screenshot::end();
                return;
            }
        };

        let res = slint::invoke_from_event_loop(move || {
            if let Err(e) = show_overlay(shot, trigger_time) {
                log::error!("Screenshot: show_overlay failed: {e}");
                destroy_overlay();
                crate::logic::screenshot::end();
            }
        });

        if let Err(e) = res {
            log::error!("Screenshot: invoke_from_event_loop failed: {e}");
            crate::logic::screenshot::end();
        }
    });
}

/// 遮罩立刻从屏幕上消失；组件本身推迟到下一轮事件循环再 drop。
/// 这个函数多半是在遮罩自己的回调里调的，当场 drop 等于在闭包跑到一半时销毁持有它的组件。
fn destroy_overlay() {
    OVERLAY.with(|cell| {
        if let Some(o) = cell.borrow().as_ref() {
            let _ = o.hide(); // ignore: 隐藏失败也要继续往下走，位子必须放开
        }
    });
    slint::Timer::single_shot(Duration::ZERO, || {
        OVERLAY.with(|cell| *cell.borrow_mut() = None);
    });
}

fn show_overlay(shot: Shot, trigger_time: Instant) -> Result<(), Error> {
    let overlay = Overlay::new()?;
    let shot_img = slint::Image::from_rgba8(shot.pixels.clone());
    overlay.set_shot(shot_img);

    let window = overlay.window();
    let w = shot.pixels.width();
    let h = shot.pixels.height();
    let (at_x, at_y) = (shot.x, shot.y);
    // 第一次 show 在屏幕外：在真实位置直接显示会播系统的开窗缩放动画。
    // 平台层拿到原生窗口后关掉这个窗口的 DWM 过渡，再把它挪到位（挪动不播动画）。
    window.set_position(PhysicalPosition::new(-32000, -32000));
    window.set_size(PhysicalSize::new(w, h));

    let shot_rc = Rc::new(shot);
    let shot_for_sel = Rc::clone(&shot_rc);

    overlay.on_selected(move |l, t, r, b| {
        // 销毁必须在裁图之前（顺序写死成一条直线，不许插 sleep）：
        // hide() → OVERLAY = None → crop_region(...) → log::info!
        destroy_overlay();
        match crate::logic::screenshot::crop_region(
            &shot_for_sel,
            l as f64,
            t as f64,
            r as f64,
            b as f64,
        ) {
            Ok(region) => {
                log::info!(
                    "Screenshot: selected rect [{}, {}, {}, {}] -> {:?}",
                    region.rect.l,
                    region.rect.t,
                    region.rect.r,
                    region.rect.b,
                    region.path,
                );
            }
            Err(e) => {
                log::error!("Screenshot: crop_region failed: {e}");
            }
        }
        crate::logic::screenshot::end();
    });

    overlay.on_cancel(move || {
        destroy_overlay();
        log::info!("Screenshot: cancelled");
        crate::logic::screenshot::end();
    });

    // Alt+F4 也当取消：不然「正在截图」的位子永远放不开，之后再也截不了图。
    let closing = overlay.as_weak();
    overlay.window().on_close_requested(move || {
        if let Some(o) = closing.upgrade() {
            o.invoke_abort();
        }
        slint::CloseRequestResponse::HideWindow
    });

    let weak = overlay.as_weak();
    OVERLAY.with(|cell| {
        *cell.borrow_mut() = Some(overlay);
    });

    OVERLAY.with(|cell| -> Result<(), Error> {
        if let Some(ref o) = *cell.borrow() {
            o.show()?;
        }
        Ok(())
    })?;

    attach_when_ready(weak, 1, trigger_time, at_x, at_y);

    Ok(())
}

fn attach_when_ready(
    weak: slint::Weak<Overlay>,
    attempt: u32,
    trigger_time: Instant,
    at_x: i32,
    at_y: i32,
) {
    const MAX_ATTEMPTS: u32 = 50;
    slint::Timer::single_shot(Duration::from_millis(10), move || {
        let Some(overlay) = weak.upgrade() else {
            return;
        };
        match platform::attach_overlay_window(overlay.window(), at_x, at_y) {
            Ok(()) => {
                let elapsed_ms = trigger_time.elapsed().as_millis();
                log::info!("Overlay: attached on attempt {attempt} ({elapsed_ms}ms since trigger)");
            }
            Err(e) if attempt >= MAX_ATTEMPTS => {
                log::error!("Overlay: attach window failed after {attempt} tries: {e}");
            }
            Err(_) => attach_when_ready(weak, attempt + 1, trigger_time, at_x, at_y),
        }
    });
}
