//! 结果浮窗（只在 UI 线程运行）。
//! 启动时建好（D12），在屏幕外 show() 一次，之后显示/隐藏全部走平台层 Win32，
//! 永远不调 Slint 的 show()/hide()（platform-windows.md §1）。

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::winit_030::WinitWindowAccessor;
use slint::winit_030::winit::event::WindowEvent;
use slint::{CloseRequestResponse, ComponentHandle, Model, PhysicalPosition, VecModel};

use crate::error::Error;
use crate::logic::config;
use crate::logic::placement::{self, Pin};
use crate::logic::popup_state::{Blur, BlurGuard};
use crate::logic::recognize;
use crate::logic::screenshot::Region;
use crate::logic::translate::{self, Query, Update};
use crate::platform::geometry::{Rect, Side};
use crate::platform::{self};
use crate::slint_ui::{EntryView, PopResult, ResultRow};
use crate::ui::entry_view;

thread_local! {
    static POP_RESULT: RefCell<Option<PopResult>> = const { RefCell::new(None) };
    static GUARD: RefCell<BlurGuard> = const { RefCell::new(BlurGuard::new()) };
    static CURRENT_QUERY: RefCell<Option<Query>> = const { RefCell::new(None) };
    static PIN: Cell<Option<Pin>> = const { Cell::new(None) };
    static MODEL: RefCell<Option<Rc<VecModel<ResultRow>>>> = const { RefCell::new(None) };
    static SPEAK_TOKEN: Cell<u64> = const { Cell::new(0) };
    /// 这次显示后第一个光标位置（物理像素），红三角的起算点；`None` = 还没动过。
    static ARM_ORIGIN: Cell<Option<(f64, f64)>> = const { Cell::new(None) };
    static VISIBLE: Cell<bool> = const { Cell::new(false) };
}

/// 鼠标离起算点超过这么远（逻辑像素）红三角才生效，防止浮窗刚出现在光标下就被碰掉（旧版 `ARM_PX`）。
const ARM_PX: f64 = 10.0;

/// 创建结果浮窗并完成初始设置。在屏幕外 show 一次并重试 attach 原生窗口。
pub fn create() -> Result<(), Error> {
    super::CREATING_INACTIVE.set(true);
    let result = PopResult::new();
    super::CREATING_INACTIVE.set(false);
    let result = result?;

    result.on_close_requested(move || {
        hide();
    });

    result.on_content_height_changed(|| {
        reposition();
    });

    let weak_speak = result.as_weak();
    result.on_speak_clicked(move |key| {
        let Some(ui) = weak_speak.upgrade() else {
            return;
        };
        handle_speak(&ui, key);
    });

    let weak_copy = result.as_weak();
    result.on_copy_clicked(move |key| {
        let Some(ui) = weak_copy.upgrade() else {
            return;
        };
        handle_copy(&ui, key);
    });

    let weak_save = result.as_weak();
    result.on_save_clicked(move |row| {
        let Some(ui) = weak_save.upgrade() else {
            return;
        };
        handle_save(&ui, row);
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
            }
        });
    });

    // Alt+F4 等关闭请求返回 KeepWindowShown，走我们自己的隐藏
    result.window().on_close_requested(|| {
        hide();
        CloseRequestResponse::KeepWindowShown
    });

    result.window().on_winit_window_event(|window, event| {
        match event {
            WindowEvent::Focused(focused) => on_focus_changed(*focused),
            // 鼠标重新进来时也补一次：切桌面回来这类情况不一定有焦点变化
            WindowEvent::CursorEntered { .. } => force_repaint(),
            WindowEvent::CursorMoved { position, .. } => arm_close(window, position.x, position.y),
            _ => {}
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

/// 光标在浮窗里移动（物理像素）。离这次显示后的第一个位置超过 `ARM_PX` 就让红三角生效。
fn arm_close(window: &slint::Window, x: f64, y: f64) {
    let Some((ox, oy)) = ARM_ORIGIN.get() else {
        ARM_ORIGIN.set(Some((x, y)));
        return;
    };
    if (x - ox).hypot(y - oy) > ARM_PX * f64::from(window.scale_factor()) {
        POP_RESULT.with_borrow(|r| {
            if let Some(ui) = r.as_ref().filter(|ui| !ui.get_close_armed()) {
                ui.set_close_armed(true);
            }
        });
    }
}

/// 翻转 `repaint-tick` 让整窗变脏重画一次（`pop_result.slint` 的背景色差）。
/// 软件渲染只提交脏区域，置顶的浮窗在用户切走/切回时系统会丢掉窗口画面，
/// 从不变化的顶栏拖动区就一直透明（platform-windows.md §1 第 4c 条）。
fn force_repaint() {
    POP_RESULT.with_borrow(|r| {
        if let Some(ui) = r.as_ref() {
            ui.set_repaint_tick(!ui.get_repaint_tick());
        }
    });
}

fn on_focus_changed(focused: bool) {
    // 切走和切回都补画：丢画面正是发生在这两个时刻前后
    force_repaint();
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

/// 隐藏结果浮窗并作废在途结果；朗读也停，否则看不见的窗口还在出声、没按钮可停。
pub fn hide() {
    GUARD.with(|g| {
        let mut guard = g.borrow_mut();
        guard.invalidate();
        guard.set_pinned(false);
    });
    POP_RESULT.with_borrow(|r| {
        if let Some(ui) = r.as_ref() {
            ui.set_is_pinned(false);
        }
    });
    VISIBLE.set(false);
    translate::invalidate();
    platform::stop_speaking();
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

        let in_place = GUARD.with(|g| g.borrow().pinned()) && VISIBLE.get();
        if in_place {
            PIN.with(|p| p.set(Some(Pin::Top)));
        }
        reset_panel(ui, in_place);
        ui.set_recognizing(false);
        ui.set_recognize_error(0);
        start_query(ui, text);

        if !in_place {
            let cfg = config::snapshot();
            place(ui, &cfg.translate.result_pos, Rect::point(x, y), 0);
            VISIBLE.set(true);
        }
    });
}

/// 框选完调入（UI 线程）：浮窗先摆在选区旁边显示「识别中」，识别在后台线程上跑（prd F2）。
/// 识别和划词共用一个 `query_id`：期间用户去划词，划词的新文字接管浮窗，这次的结果丢掉。
pub fn show_recognizing(region: Region) {
    let id = translate::invalidate();
    POP_RESULT.with(|r| {
        let binding = r.borrow();
        let Some(ui) = binding.as_ref() else {
            log::warn!("PopResult: window not available");
            return;
        };

        let in_place = GUARD.with(|g| g.borrow().pinned()) && VISIBLE.get();
        if in_place {
            PIN.with(|p| p.set(Some(Pin::Top)));
        }
        reset_panel(ui, in_place);
        ui.set_recognizing(true);
        ui.set_recognize_error(0);
        ui.set_source_text("".into());
        ui.set_rows(Rc::new(VecModel::<ResultRow>::default()).into());
        MODEL.with(|m| *m.borrow_mut() = None);
        CURRENT_QUERY.with(|q| *q.borrow_mut() = None);

        if !in_place {
            let cfg = config::snapshot();
            place(ui, &cfg.translate.result_pos, region.rect, 4);
            VISIBLE.set(true);
        }
    });

    // 临时图交给 Drop 删：识别线程起不来、或者识别中途 panic，文件照样清掉
    // （直接写在闭包末尾的话这两条路都会跳过它，`ocr_region_*.png` 就留在缓存目录里了，B3 审查）。
    let temp = TempImage(region.path);
    if let Err(e) = std::thread::Builder::new()
        .name("recognize".into())
        .spawn(move || {
            // 删除只发生在这之后：识别函数已经返回，谁也不在读它了。
            let outcome = recognize::run_for_ui(&temp.0);
            drop(temp);
            // ignore: 事件循环没了就没人显示结果了
            let _ = slint::invoke_from_event_loop(move || on_recognized(id, outcome));
        })
    {
        log::error!("PopResult: spawn recognize thread failed: {e}");
        on_recognized(id, Err(recognize::FAILED));
    }
}

/// 一张截图临时图，离开作用域就删。结果作废（用户又截了一张 / 去划词）也照删。
struct TempImage(std::path::PathBuf);

impl Drop for TempImage {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_file(&self.0) {
            log::warn!("Recognize: remove temp image failed: {e}");
        }
    }
}

/// 识别有结果了（UI 线程）。这期间用户可能已经划了词或又截了一张图，
/// 所以要再比一次代次：不是当前那次的结果直接丢掉（prd F2）。
fn on_recognized(id: u64, outcome: Result<String, i32>) {
    if id != translate::current() {
        log::info!("Recognize: stale result discarded");
        return;
    }
    POP_RESULT.with(|r| {
        let binding = r.borrow();
        let Some(ui) = binding.as_ref() else { return };
        ui.set_recognizing(false);
        match outcome {
            Ok(text) => start_query(ui, &text),
            Err(code) => ui.set_recognize_error(code),
        }
    });
}

/// 按用户选的位置算出 (anchor, sx, sy, gap)。
/// `sel` 是「相对选词」用的锚点：划词链路传浮标那个零宽点，截图链路传选区矩形。
/// `sel_gap` 同理：划词 0，截图 4（旧版 GAP）。
/// `bounds` 是目标显示器的工作区，只有 screen_* 用得到。
fn anchor_for(pos: &str, sel: Rect, sel_gap: i32, bounds: Rect) -> (Rect, Side, Side, i32) {
    match pos {
        "sel_bottom" => (sel, Side::Start, Side::After, sel_gap),
        "sel_top" => (sel, Side::Start, Side::Before, sel_gap),
        "sel_left" => (sel, Side::Before, Side::Start, sel_gap),
        "sel_right" => (sel, Side::After, Side::Start, sel_gap),
        "cursor_top_left" => {
            let (x, y) = platform::cursor_pos();
            (Rect::point(x, y), Side::Before, Side::Before, 0)
        }
        "cursor_top_right" => {
            let (x, y) = platform::cursor_pos();
            (Rect::point(x, y), Side::After, Side::Before, 0)
        }
        "cursor_bottom_left" => {
            let (x, y) = platform::cursor_pos();
            (Rect::point(x, y), Side::Before, Side::After, 0)
        }
        "cursor_bottom_right" => {
            let (x, y) = platform::cursor_pos();
            (Rect::point(x, y), Side::After, Side::After, 0)
        }
        "screen_top_left" => (Rect::point(bounds.l, bounds.t), Side::Start, Side::Start, 0),
        "screen_top_right" => (Rect::point(bounds.r, bounds.t), Side::End, Side::Start, 0),
        "screen_bottom_left" => (Rect::point(bounds.l, bounds.b), Side::Start, Side::End, 0),
        "screen_bottom_right" => (Rect::point(bounds.r, bounds.b), Side::End, Side::End, 0),
        _ => (sel, Side::Start, Side::After, sel_gap),
    }
}

/// 每次显示前的复位：焦点看护、朗读、各按钮状态、红三角起算点。
/// 置顶原地刷新时不重置置顶状态。
fn reset_panel(ui: &PopResult, in_place: bool) {
    if in_place {
        GUARD.with(|g| g.borrow_mut().invalidate());
    } else {
        GUARD.with(|g| g.borrow_mut().begin(Instant::now()));
        ui.set_is_pinned(false);
    }
    ui.set_color_scheme(super::resolve_color_scheme(ui.window()));
    platform::stop_speaking();
    SPEAK_TOKEN.with(|t| t.set(t.get().wrapping_add(1)));
    ui.set_speaking_key(-2);
    ui.set_copied_key(-2);
    ui.set_saved_row(-1);
    ui.set_lang_badge("".into());
    ui.set_close_armed(false);
    ARM_ORIGIN.set(None);
}

/// 发起一次翻译并把面板换成它的行。截图那条路在识别出字之后才走到这里，
/// 浮窗已经在屏幕上了，所以这里不摆位置 —— 高度变了由 `content-height-changed` 接着调。
fn start_query(ui: &PopResult, text: &str) {
    let query = translate::start(text, move |id, update| {
        if let Err(e) = slint::invoke_from_event_loop(move || {
            on_translate_update(id, update);
        }) {
            log::warn!("PopResult: invoke update from event loop failed: {e}");
        }
    });

    let single_line_source = query.text.split_whitespace().collect::<Vec<_>>().join(" ");
    ui.set_source_text(single_line_source.as_str().into());

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
            display: EntryView::default(),
        })
        .collect();
    let model = Rc::new(VecModel::from(row_items));
    ui.set_rows(model.clone().into());
    MODEL.with(|m| *m.borrow_mut() = Some(model));
    CURRENT_QUERY.with(|q| *q.borrow_mut() = Some(query));
}

/// 按当前内容高度算出位置并把浮窗亮出来。
fn place(ui: &PopResult, pos: &str, sel: Rect, sel_gap: i32) {
    let (cx, cy) = if pos.starts_with("cursor_") {
        platform::cursor_pos()
    } else {
        (sel.l + (sel.r - sel.l) / 2, sel.t + (sel.b - sel.t) / 2)
    };
    let Some((bounds, scale)) = platform::monitor_at(cx, cy) else {
        log::warn!("PopResult: cannot find monitor at ({cx}, {cy})");
        return;
    };
    let (anchor, sx, sy, gap) = anchor_for(pos, sel, sel_gap, bounds);
    let content_height = ui.get_content_height();
    let Some((rect, mut pin)) =
        placement::first_placement(anchor, sx, sy, gap, content_height, scale, bounds)
    else {
        log::warn!("PopResult: first_placement calculation failed");
        return;
    };

    if matches!(pos, "screen_bottom_left" | "screen_bottom_right") {
        pin = Pin::Bottom(rect.b);
    }

    PIN.with(|p| p.set(Some(pin)));
    platform::show_result_window(rect);
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
        Update::Detected(code) => {
            let lang = crate::logic::config::snapshot().general.language;
            let badge = lang_badge(code, &lang);
            POP_RESULT.with(|r| {
                if let Some(ui) = r.borrow().as_ref() {
                    ui.set_lang_badge(badge.into());
                }
            });
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
                            r.display = entry_view::to_view(&shown.display, &shown.text);
                            r.error = "".into();
                        }
                        Err(e) => {
                            r.text = "".into();
                            r.display = EntryView::default();
                            r.error = e.to_string().into();
                        }
                    }
                    model.set_row_data(row, r);
                }
            });
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

/// 按钮编码对应的文字：`-1` = 原文，`>= 0` = 该行结果。空文字当没有。
fn text_for(key: i32) -> Option<String> {
    let text = if key == -1 {
        CURRENT_QUERY.with(|q| q.borrow().as_ref().map(|query| query.text.clone()))
    } else {
        let row = usize::try_from(key).ok()?;
        MODEL.with(|m| {
            m.borrow()
                .as_ref()
                .and_then(|model| model.row_data(row))
                .map(|r| r.text.to_string())
        })
    };
    text.filter(|t| !t.is_empty())
}

fn handle_speak(ui: &PopResult, key: i32) {
    let Some(text) = text_for(key) else { return };

    let current_key = ui.get_speaking_key();
    if current_key == key {
        platform::stop_speaking();
        SPEAK_TOKEN.with(|t| t.set(t.get().wrapping_add(1)));
        ui.set_speaking_key(-2);
    } else {
        let next_token = SPEAK_TOKEN.with(|t| {
            let val = t.get().wrapping_add(1);
            t.set(val);
            val
        });
        let weak_ui = ui.as_weak();
        let voice = crate::logic::result::voice_for(&text);
        let res = platform::speak(&text, voice, move || {
            if let Err(e) = slint::invoke_from_event_loop(move || {
                let matches_token = SPEAK_TOKEN.with(|t| t.get() == next_token);
                if matches_token && let Some(ui) = weak_ui.upgrade() {
                    ui.set_speaking_key(-2);
                }
            }) {
                log::warn!("PopResult: speak done invoke failed: {e}");
            }
        });
        match res {
            Ok(()) => {
                ui.set_speaking_key(key);
            }
            Err(e) => {
                log::warn!("PopResult: speak failed: {e}");
            }
        }
    }
}

fn handle_copy(ui: &PopResult, key: i32) {
    let Some(text) = text_for(key) else { return };

    match platform::copy_text(&text) {
        Ok(()) => {
            ui.set_copied_key(key);
            let weak = ui.as_weak();
            slint::Timer::single_shot(Duration::from_millis(1500), move || {
                if let Some(ui) = weak.upgrade()
                    && ui.get_copied_key() == key
                {
                    ui.set_copied_key(-2);
                }
            });
        }
        Err(e) => {
            log::warn!("PopResult: copy text failed: {e}");
        }
    }
}

fn handle_save(ui: &PopResult, row: i32) {
    if row < 0 || ui.get_saved_row() == row {
        return;
    }
    let row_idx = row as usize;
    let weak_ui = ui.as_weak();
    CURRENT_QUERY.with(|q| {
        let binding = q.borrow();
        let Some(query) = binding.as_ref() else {
            return;
        };
        let query_id = query.id;
        query.save(Some(row_idx), move |ok| {
            let weak = weak_ui.clone();
            if let Err(e) = slint::invoke_from_event_loop(move || {
                let is_current = CURRENT_QUERY
                    .with(|q| q.borrow().as_ref().is_some_and(|cur| cur.id == query_id));
                if !is_current {
                    return;
                }
                let Some(ui) = weak.upgrade() else { return };
                if ok {
                    ui.set_saved_row(row);
                } else {
                    log::warn!("PopResult: save to wordbook failed");
                    if ui.get_saved_row() == row {
                        ui.set_saved_row(-1);
                    }
                }
            }) {
                log::warn!("PopResult: save invoke from event loop failed: {e}");
            }
        });
    });
}

/// 语种识别代码转徽标文字。
/// 中文界面：`zh_cn`→中、`zh_tw`→繁、`en`→英、`ja`→日、`ko`→韩；
/// 英文界面：`zh_cn`→ZH、`zh_tw`→TW、`en`→EN、`ja`→JA、`ko`→KO；
/// 其他取 `_` 前的部分转大写（`fr`→FR、`pt_pt`→PT）；`None`（检测失败）→ 空，不伪造徽标。
fn lang_badge(code: Option<&str>, ui_lang: &str) -> String {
    let Some(code) = code else {
        return String::new();
    };

    // 英文界面只有 zh_tw 要单列：通用分支会把它和 zh_cn 一样给成 ZH，繁简就分不开了。
    if ui_lang == "en" && code == "zh_tw" {
        return "TW".into();
    }
    if ui_lang != "en" {
        match code {
            "zh_cn" => return "中".into(),
            "zh_tw" => return "繁".into(),
            "en" => return "英".into(),
            "ja" => return "日".into(),
            "ko" => return "韩".into(),
            _ => {}
        }
    }

    code.split('_').next().unwrap_or(code).to_ascii_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anchor_for_all_positions() {
        let sel = Rect {
            l: 100,
            t: 200,
            r: 300,
            b: 400,
        };
        let bounds = Rect {
            l: 0,
            t: 0,
            r: 1920,
            b: 1080,
        };

        // 1. sel_bottom: (Start, After), gap
        let (a, sx, sy, gap) = anchor_for("sel_bottom", sel, 0, bounds);
        assert_eq!((a, sx, sy, gap), (sel, Side::Start, Side::After, 0));
        let (a, sx, sy, gap) = anchor_for("sel_bottom", sel, 4, bounds);
        assert_eq!((a, sx, sy, gap), (sel, Side::Start, Side::After, 4));

        // 2. sel_top: (Start, Before), gap
        let (a, sx, sy, gap) = anchor_for("sel_top", sel, 0, bounds);
        assert_eq!((a, sx, sy, gap), (sel, Side::Start, Side::Before, 0));
        let (a, sx, sy, gap) = anchor_for("sel_top", sel, 4, bounds);
        assert_eq!((a, sx, sy, gap), (sel, Side::Start, Side::Before, 4));

        // 3. sel_left: (Before, Start), gap
        let (a, sx, sy, gap) = anchor_for("sel_left", sel, 0, bounds);
        assert_eq!((a, sx, sy, gap), (sel, Side::Before, Side::Start, 0));
        let (a, sx, sy, gap) = anchor_for("sel_left", sel, 4, bounds);
        assert_eq!((a, sx, sy, gap), (sel, Side::Before, Side::Start, 4));

        // 4. sel_right: (After, Start), gap
        let (a, sx, sy, gap) = anchor_for("sel_right", sel, 0, bounds);
        assert_eq!((a, sx, sy, gap), (sel, Side::After, Side::Start, 0));
        let (a, sx, sy, gap) = anchor_for("sel_right", sel, 4, bounds);
        assert_eq!((a, sx, sy, gap), (sel, Side::After, Side::Start, 4));

        // 5. cursor_top_left: (Before, Before), 0
        let (a, sx, sy, gap) = anchor_for("cursor_top_left", sel, 0, bounds);
        assert_eq!((sx, sy, gap), (Side::Before, Side::Before, 0));
        assert_eq!(a.l, a.r);
        assert_eq!(a.t, a.b);

        // 6. cursor_top_right: (After, Before), 0
        let (a, sx, sy, gap) = anchor_for("cursor_top_right", sel, 0, bounds);
        assert_eq!((sx, sy, gap), (Side::After, Side::Before, 0));
        assert_eq!(a.l, a.r);
        assert_eq!(a.t, a.b);

        // 7. cursor_bottom_left: (Before, After), 0
        let (a, sx, sy, gap) = anchor_for("cursor_bottom_left", sel, 0, bounds);
        assert_eq!((sx, sy, gap), (Side::Before, Side::After, 0));
        assert_eq!(a.l, a.r);
        assert_eq!(a.t, a.b);

        // 8. cursor_bottom_right: (After, After), 0
        let (a, sx, sy, gap) = anchor_for("cursor_bottom_right", sel, 0, bounds);
        assert_eq!((sx, sy, gap), (Side::After, Side::After, 0));
        assert_eq!(a.l, a.r);
        assert_eq!(a.t, a.b);

        // 9. screen_top_left: (Start, Start), 0, anchor = point(bounds.l, bounds.t) -> (0, 0)
        let (a, sx, sy, gap) = anchor_for("screen_top_left", sel, 0, bounds);
        assert_eq!(
            (a, sx, sy, gap),
            (
                Rect {
                    l: 0,
                    t: 0,
                    r: 0,
                    b: 0
                },
                Side::Start,
                Side::Start,
                0
            )
        );

        // 10. screen_top_right: (End, Start), 0, anchor = point(bounds.r, bounds.t) -> (1920, 0)
        let (a, sx, sy, gap) = anchor_for("screen_top_right", sel, 0, bounds);
        assert_eq!(
            (a, sx, sy, gap),
            (
                Rect {
                    l: 1920,
                    t: 0,
                    r: 1920,
                    b: 0
                },
                Side::End,
                Side::Start,
                0
            )
        );

        // 11. screen_bottom_left: (Start, End), 0, anchor = point(bounds.l, bounds.b) -> (0, 1080)
        let (a, sx, sy, gap) = anchor_for("screen_bottom_left", sel, 0, bounds);
        assert_eq!(
            (a, sx, sy, gap),
            (
                Rect {
                    l: 0,
                    t: 1080,
                    r: 0,
                    b: 1080
                },
                Side::Start,
                Side::End,
                0
            )
        );

        // 12. screen_bottom_right: (End, End), 0, anchor = point(bounds.r, bounds.b) -> (1920, 1080)
        let (a, sx, sy, gap) = anchor_for("screen_bottom_right", sel, 0, bounds);
        assert_eq!(
            (a, sx, sy, gap),
            (
                Rect {
                    l: 1920,
                    t: 1080,
                    r: 1920,
                    b: 1080
                },
                Side::End,
                Side::End,
                0
            )
        );

        // 13. unrecognized value falls back to sel_bottom
        let (a, sx, sy, gap) = anchor_for("unknown_pos", sel, 4, bounds);
        assert_eq!((a, sx, sy, gap), (sel, Side::Start, Side::After, 4));
    }

    #[test]
    fn lang_badge_mapping() {
        assert_eq!(lang_badge(Some("zh_cn"), "zh_CN"), "中");
        assert_eq!(lang_badge(Some("zh_tw"), "zh_CN"), "繁");
        assert_eq!(lang_badge(Some("en"), "zh_CN"), "英");
        assert_eq!(lang_badge(Some("ja"), "zh_CN"), "日");
        assert_eq!(lang_badge(Some("ko"), "zh_CN"), "韩");
        assert_eq!(lang_badge(Some("fr"), "zh_CN"), "FR");
        assert_eq!(lang_badge(Some("pt_pt"), "zh_CN"), "PT");
        assert_eq!(lang_badge(None, "zh_CN"), "");
    }

    #[test]
    fn lang_badge_mapping_en() {
        assert_eq!(lang_badge(Some("zh_cn"), "en"), "ZH");
        assert_eq!(lang_badge(Some("zh_tw"), "en"), "TW");
        assert_eq!(lang_badge(Some("en"), "en"), "EN");
        assert_eq!(lang_badge(Some("ja"), "en"), "JA");
        assert_eq!(lang_badge(Some("ko"), "en"), "KO");
        assert_eq!(lang_badge(Some("fr"), "en"), "FR");
        assert_eq!(lang_badge(Some("pt_pt"), "en"), "PT");
        assert_eq!(lang_badge(None, "en"), "");
    }
}
