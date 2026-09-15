//! 结果浮窗（只在 UI 线程运行）。
//! 启动时建好（D12），在屏幕外 show() 一次，之后显示/隐藏全部走平台层 Win32，
//! 永远不调 Slint 的 show()/hide()（platform-windows.md §1）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::language::ColorScheme;
use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::event::WindowEvent;
use slint::winit_030::winit::window::Theme as WinitTheme;
use slint::{CloseRequestResponse, ComponentHandle, Model, PhysicalPosition, VecModel};

use crate::error::Error;
use crate::logic::config;
use crate::logic::placement::{self, Pin};
use crate::logic::popup_state::{Blur, BlurGuard};
use crate::logic::translate::{self, Query, Update};
use crate::platform::geometry::{Rect, Side};
use crate::platform::{self};
use crate::slint_ui::{PopResult, ResultRow};
use crate::ui::pop_button;

thread_local! {
    static POP_RESULT: RefCell<Option<PopResult>> = const { RefCell::new(None) };
    static GUARD: RefCell<BlurGuard> = const { RefCell::new(BlurGuard::new()) };
    static CURRENT_QUERY: RefCell<Option<Query>> = const { RefCell::new(None) };
    static PIN: Cell<Option<Pin>> = const { Cell::new(None) };
    static MODEL: RefCell<Option<Rc<VecModel<ResultRow>>>> = const { RefCell::new(None) };
}

/// 创建结果浮窗并完成初始设置。在屏幕外 show 一次并重试 attach 原生窗口。
pub fn create() -> Result<(), Error> {
    super::CREATING_INACTIVE.set(true);
    let result = PopResult::new();
    super::CREATING_INACTIVE.set(false);
    let result = result?;

    result.on_close_requested(move || {
        hide();
    });

    let weak_pin = result.as_weak();
    result.on_pin_toggled(move || {
        let Some(ui) = weak_pin.upgrade() else { return };
        GUARD.with(|g| {
            let mut guard = g.borrow_mut();
            let next = !guard.pinned();
            guard.set_pinned(next);
            ui.set_is_pinned(next);
        });
    });

    result.on_drag_started(|| {
        PIN.with(|p| p.set(Some(Pin::Top)));
    });

    let weak_drag = result.as_weak();
    result.on_dragged(move |dx, dy| {
        let Some(ui) = weak_drag.upgrade() else {
            return;
        };
        let window = ui.window();
        let s = window.scale_factor();
        let p = window.position();
        window.set_position(PhysicalPosition::new(
            p.x + (dx * s).round() as i32,
            p.y + (dy * s).round() as i32,
        ));
    });

    result.on_toggle_collapse(move |idx| {
        let idx = idx as usize;
        MODEL.with(|m| {
            let binding = m.borrow();
            let Some(model) = binding.as_ref() else {
                return;
            };
            if let Some(mut row) = model.row_data(idx) {
                row.collapsed = !row.collapsed;
                model.set_row_data(idx, row);
                reposition();
            }
        });
    });

    // Alt+F4 等关闭请求返回 KeepWindowShown，走我们自己的隐藏
    result.window().on_close_requested(|| {
        hide();
        CloseRequestResponse::KeepWindowShown
    });

    result.window().on_winit_window_event(|_window, event| {
        if let WindowEvent::Focused(focused) = event {
            on_focus_changed(*focused);
        }
        slint::winit_030::EventResult::Propagate
    });

    // 第一次 show 在屏幕外：原生窗口建出来、交给平台层之前那一下不会闪到屏幕上。
    result
        .window()
        .set_position(PhysicalPosition::new(-32000, -32000));
    result.show()?;
    attach_when_ready(result.as_weak(), 1);

    POP_RESULT.with(|r| *r.borrow_mut() = Some(result));
    Ok(())
}

fn attach_when_ready(weak: slint::Weak<PopResult>, attempt: u32) {
    const MAX_ATTEMPTS: u32 = 50;
    slint::Timer::single_shot(Duration::from_millis(10), move || {
        let Some(result) = weak.upgrade() else { return };
        match platform::attach_result_window(result.window()) {
            Ok(()) => {}
            Err(e) if attempt >= MAX_ATTEMPTS => {
                log::error!("PopResult: attach window failed after {attempt} tries: {e}");
            }
            Err(_) => attach_when_ready(weak, attempt + 1),
        }
    });
}

fn on_focus_changed(focused: bool) {
    if focused {
        GUARD.with(|g| g.borrow_mut().focus());
    } else {
        let now = Instant::now();
        let action = GUARD.with(|g| g.borrow_mut().blur(now));
        match action {
            Blur::Ignore => {}
            Blur::Hide => hide(),
            Blur::CheckLater { after, token } => {
                slint::Timer::single_shot(after, move || {
                    let should_hide = GUARD.with(|g| {
                        g.borrow_mut()
                            .check(token, platform::result_window_focused())
                    });
                    if should_hide {
                        hide();
                    }
                });
            }
        }
    }
}

/// 隐藏结果浮窗并作废在途结果。
pub fn hide() {
    GUARD.with(|g| g.borrow_mut().invalidate());
    translate::invalidate();
    platform::hide_result_window();
}

/// 浮标触发后调入：重置状态、发起翻译、算初次摆放并显示浮窗。
pub fn show(text: &str, x: i32, y: i32) {
    POP_RESULT.with(|r| {
        let binding = r.borrow();
        let Some(ui) = binding.as_ref() else {
            log::warn!("PopResult: window not available");
            return;
        };

        GUARD.with(|g| g.borrow_mut().begin(Instant::now()));

        let cfg = config::snapshot();
        let scheme = match cfg.general.theme.as_str() {
            "dark" => ColorScheme::Dark,
            "light" => ColorScheme::Light,
            _ => match ui.window().with_winit_window(|w| w.theme()).flatten() {
                Some(WinitTheme::Dark) => ColorScheme::Dark,
                Some(WinitTheme::Light) => ColorScheme::Light,
                // 拿不到系统主题时回退 Unknown：fluent 走系统默认，自绘部分因 dark 判断为 false 按亮色显示（已知限制）
                None => ColorScheme::Unknown,
            },
        };
        ui.set_color_scheme(scheme);

        let query = translate::start(text, move |id, update| {
            if let Err(e) = slint::invoke_from_event_loop(move || {
                on_translate_update(id, update);
            }) {
                log::warn!("PopResult: invoke update from event loop failed: {e}");
            }
        });

        ui.set_source_text(query.text.as_str().into());
        ui.set_is_pinned(false);

        let row_items: Vec<ResultRow> = query
            .rows
            .iter()
            .map(|r| ResultRow {
                service_id: r.service_id.as_str().into(),
                kind: r.kind.into(),
                label: r.label.as_str().into(),
                collapsed: false,
                loading: true,
                text: "".into(),
                error: "".into(),
            })
            .collect();
        let model = Rc::new(VecModel::from(row_items));
        ui.set_rows(model.clone().into());
        MODEL.with(|m| *m.borrow_mut() = Some(model));
        CURRENT_QUERY.with(|q| *q.borrow_mut() = Some(query));

        let content_height = ui.get_content_height();
        let Some((bounds, scale)) = platform::monitor_at(x, y) else {
            log::warn!("PopResult: cannot find monitor at ({x}, {y})");
            return;
        };

        let (sx, sy) =
            pop_button::corner(&cfg.selection.result_pos).unwrap_or((Side::After, Side::After));
        let anchor = Rect::point(x, y);
        let gap = 0;
        let Some((rect, pin)) =
            placement::first_placement(anchor, sx, sy, gap, content_height, scale, bounds)
        else {
            log::warn!("PopResult: first_placement calculation failed");
            return;
        };

        PIN.with(|p| p.set(Some(pin)));
        platform::show_result_window(rect);
    });
}

fn on_translate_update(id: u64, update: Update) {
    if id != translate::current() {
        return;
    }
    let is_current =
        CURRENT_QUERY.with(|q| q.borrow().as_ref().is_some_and(|query| query.id == id));
    if !is_current {
        return;
    }

    match update {
        Update::Detected(_) => {
            // 本任务忽略语种徽标更新（下个任务做）
        }
        Update::Done { row, result } => {
            MODEL.with(|m| {
                let binding = m.borrow();
                let Some(model) = binding.as_ref() else {
                    return;
                };
                if let Some(mut r) = model.row_data(row) {
                    r.loading = false;
                    match result {
                        Ok(shown) => {
                            r.text = shown.text.as_str().into();
                            r.error = "".into();
                        }
                        Err(e) => {
                            r.text = "".into();
                            r.error = e.to_string().into();
                        }
                    }
                    model.set_row_data(row, r);
                }
            });
            reposition();
        }
    }
}

fn reposition() {
    POP_RESULT.with(|r| {
        let binding = r.borrow();
        let Some(ui) = binding.as_ref() else { return };
        let pin = PIN.with(|p| p.get());
        let Some(pin) = pin else { return };

        let win = ui.window();
        let pos = win.position();
        let size = win.size();
        let current = Rect {
            l: pos.x,
            t: pos.y,
            r: pos.x + size.width as i32,
            b: pos.y + size.height as i32,
        };
        let cx = current.l + (current.r - current.l) / 2;
        let cy = current.t + (current.b - current.t) / 2;

        let Some((bounds, scale)) = platform::monitor_at(cx, cy) else {
            return;
        };
        let content_height = ui.get_content_height();
        if let Some(new_rect) = placement::place_result(current, pin, content_height, scale, bounds)
            && new_rect != current
        {
            platform::move_result_window(new_rect);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corner_defaults_match_requirements() {
        assert_eq!(
            pop_button::corner("BottomRight"),
            Some((Side::After, Side::After))
        );
        assert_eq!(
            pop_button::corner("BottomLeft"),
            Some((Side::Before, Side::After))
        );
        assert_eq!(
            pop_button::corner("TopRight"),
            Some((Side::After, Side::Before))
        );
        assert_eq!(
            pop_button::corner("TopLeft"),
            Some((Side::Before, Side::Before))
        );
        assert_eq!(pop_button::corner("unknown"), None);
    }
}
