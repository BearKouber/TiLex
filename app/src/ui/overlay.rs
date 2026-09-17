//! 截图遮罩：全屏冻结画面 + 拖框选区。
//!
//! 启动时建好、常驻，显示/隐藏全部走平台层的 DWM cloak（design §1.4）。
//! **永远不调它的 `show()`/`hide()`**：那会走 `ShowWindow`，Windows 给它播约 200ms 的
//! 开窗过渡，遮罩从屏幕中心缩放着展开（B3 第 2 轮手测）。
//! 内存不因此常驻：平时窗口尺寸压到 1×1，截图时才 resize 到整个虚拟屏，
//! 关掉后 resize 回去并清空 `shot`，截图缓冲和帧缓冲都立刻释放。

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, PhysicalPosition, PhysicalSize};

use crate::error::Error;
use crate::platform::geometry::Rect;
use crate::platform::{self, Shot};
use crate::slint_ui::Overlay;

thread_local! {
    static OVERLAY: RefCell<Option<Overlay>> = const { RefCell::new(None) };
    /// 正在框选的那一帧。选完要从它裁图，所以不能只交给界面。
    static SHOT: RefCell<Option<Rc<Shot>>> = const { RefCell::new(None) };
}

/// 隐藏时的窗口尺寸。0 会被 winit 拒绝，用 1×1：帧缓冲几乎为零。
const HIDDEN_SIZE: PhysicalSize = PhysicalSize {
    width: 1,
    height: 1,
};

/// 建遮罩窗口并藏起来。启动时调一次（UI 线程）。
pub fn create() -> Result<(), Error> {
    // 启动时不许抢用户的焦点；真正显示时由平台层 force_foreground 抢。
    super::CREATING_INACTIVE.set(true);
    let overlay = Overlay::new();
    super::CREATING_INACTIVE.set(false);
    let overlay = overlay?;

    overlay.on_selected(|l, t, r, b| {
        // 顺序写死成一条直线，不许插 sleep：遮罩先消失、焦点还给原程序，再裁图。
        hide();
        let Some(shot) = SHOT.with(|s| s.borrow_mut().take()) else {
            log::error!("Screenshot: no frame to crop");
            crate::logic::screenshot::end();
            return;
        };
        match crate::logic::screenshot::crop_region(&shot, l.into(), t.into(), r.into(), b.into()) {
            Ok(region) => log::info!(
                "Screenshot: selected rect [{}, {}, {}, {}] -> {:?}",
                region.rect.l,
                region.rect.t,
                region.rect.r,
                region.rect.b,
                region.path,
            ),
            Err(e) => log::error!("Screenshot: crop_region failed: {e}"),
        }
        crate::logic::screenshot::end();
    });

    overlay.on_cancel(|| {
        hide();
        SHOT.with(|s| *s.borrow_mut() = None);
        log::info!("Screenshot: cancelled");
        crate::logic::screenshot::end();
    });

    // Alt+F4 当取消：不然「正在截图」的位子永远放不开，之后再也截不了图。
    let closing = overlay.as_weak();
    overlay.window().on_close_requested(move || {
        if let Some(o) = closing.upgrade() {
            o.invoke_abort();
        }
        slint::CloseRequestResponse::KeepWindowShown
    });

    // 唯一一次 show()：在屏幕外，那一次的开窗动画没人看得见。之后只 cloak。
    overlay
        .window()
        .set_position(PhysicalPosition::new(-32000, -32000));
    overlay.window().set_size(HIDDEN_SIZE);
    overlay.show()?;
    attach_when_ready(overlay.as_weak(), 1);
    OVERLAY.with(|cell| *cell.borrow_mut() = Some(overlay));
    Ok(())
}

fn attach_when_ready(weak: slint::Weak<Overlay>, attempt: u32) {
    const MAX_ATTEMPTS: u32 = 50;
    slint::Timer::single_shot(Duration::from_millis(10), move || {
        let Some(overlay) = weak.upgrade() else {
            return;
        };
        match platform::attach_overlay_window(overlay.window()) {
            Ok(()) => {}
            Err(e) if attempt >= MAX_ATTEMPTS => {
                log::error!("Overlay: attach window failed after {attempt} tries: {e}");
            }
            Err(_) => attach_when_ready(weak, attempt + 1),
        }
    });
}

/// 开一次截图。任何线程都能调（快捷键线程、托盘在 UI 线程）。
/// 抓屏几十毫秒，所以永远在自己起的线程上抓（R-5），抓完切回 UI 线程显示遮罩。
pub fn start() {
    if !crate::logic::screenshot::begin() {
        log::info!("Screenshot: already in progress, ignored");
        return;
    }
    let started = Instant::now();
    std::thread::spawn(move || {
        let shot = match platform::capture_screen() {
            Ok(s) => s,
            Err(e) => {
                log::error!("Screenshot: capture_screen failed: {e}");
                crate::logic::screenshot::end();
                return;
            }
        };
        if let Err(e) = slint::invoke_from_event_loop(move || show(shot, started)) {
            log::error!("Screenshot: event loop gone: {e}");
            crate::logic::screenshot::end();
        }
    });
}

/// 显示遮罩（UI 线程）。窗口早就建好了，这里只是填内容、撑到整个虚拟屏、解除 cloak。
fn show(shot: Shot, started: Instant) {
    let (w, h) = (shot.pixels.width(), shot.pixels.height());
    let at = Rect {
        l: shot.x,
        t: shot.y,
        r: shot.x + w as i32,
        b: shot.y + h as i32,
    };
    let shown = OVERLAY.with(|cell| {
        let borrow = cell.borrow();
        let Some(overlay) = borrow.as_ref() else {
            return false;
        };
        overlay.invoke_reset();
        overlay.set_shot(slint::Image::from_rgba8(shot.pixels.clone()));
        overlay.window().set_size(PhysicalSize::new(w, h));
        // 软件渲染只画脏区域；窗口刚从 1×1 撑到全屏，不整窗重画会露出上一帧
        // （platform-windows.md §1 第 4 条，浮标踩过同样的坑）。
        overlay.window().request_redraw();
        true
    });
    if !shown {
        log::error!("Screenshot: overlay window is not ready");
        crate::logic::screenshot::end();
        return;
    }
    SHOT.with(|s| *s.borrow_mut() = Some(Rc::new(shot)));
    platform::show_overlay_window(at);
    log::info!(
        "Overlay: shown {}ms after trigger",
        started.elapsed().as_millis()
    );
}

/// 遮罩从屏幕上消失，并把那两份大缓冲还回去（UI 线程）。
fn hide() {
    platform::hide_overlay_window();
    OVERLAY.with(|cell| {
        if let Some(overlay) = cell.borrow().as_ref() {
            overlay.set_shot(slint::Image::default());
            overlay.window().set_size(HIDDEN_SIZE);
        }
    });
}
