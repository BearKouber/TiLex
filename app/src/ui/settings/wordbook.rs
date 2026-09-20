//! 设置窗口：生词本页（列表、搜索、筛选、右卡详情、发音、多选批量删除、单条删除、导出 Markdown、变更自动刷新）。

use slint::{ComponentHandle, Model, ModelRc, VecModel};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::logic::result::{self, DisplayKind, Kind};
use crate::logic::wordbook::{self, Entry, Summary};
use crate::logic::wordbook_export;
use crate::platform;
use crate::slint_ui::{EntryView, SettingsWindow, WordbookRow, WordbookState};

struct WordbookUiState {
    entries: Vec<Summary>,
    selected_id: i64,
    speak_token: u64,
    speaking: bool,
    batch_mode: bool,
    checked: HashSet<i64>,
    deleting: bool,
    pending: Vec<i64>,
    /// `pending` 是多选那条路来的还是右卡单条删来的。
    /// 旧版 `Wordbook/index.jsx:122` 的 `remove(ids, batch)`：只有批量删完才退出多选，
    /// 多选模式下删单条只把它从勾选集合里摘掉，用户的其余勾选要留着。
    pending_is_batch: bool,
    refresh_pending: bool,
    visible_ids: Vec<i64>,
    rows_model: Option<Rc<VecModel<WordbookRow>>>,
}

thread_local! {
    // 不能写 const {}：`HashSet::new` 不是 const fn（要建 RandomState）
    static STATE: RefCell<WordbookUiState> = RefCell::new(WordbookUiState {
        entries: Vec::new(),
        selected_id: 0,
        speak_token: 0,
        speaking: false,
        batch_mode: false,
        checked: HashSet::new(),
        deleting: false,
        pending: Vec::new(),
        pending_is_batch: false,
        refresh_pending: false,
        visible_ids: Vec::new(),
        rows_model: None,
    });
}

pub fn bind(page: &SettingsWindow) {
    STATE.with_borrow_mut(|s| {
        s.entries.clear();
        s.selected_id = 0;
        s.speak_token = s.speak_token.wrapping_add(1);
        s.speaking = false;
        s.batch_mode = false;
        s.checked.clear();
        s.deleting = false;
        s.pending.clear();
        s.pending_is_batch = false;
        s.refresh_pending = false;
        s.visible_ids.clear();
        s.rows_model = None;
    });

    let state = page.global::<WordbookState>();
    state.set_rows(ModelRc::new(VecModel::default()));
    state.set_selected_id(0);
    state.set_detail_text("".into());
    state.set_detail_is_word(false);
    state.set_detail_category("".into());
    state.set_detail_difficulty(0);
    state.set_detail_difficulty_reason("".into());
    state.set_detail_entry(EntryView::default());
    state.set_speaking(false);
    state.set_keyword("".into());
    state.set_filter(0);

    state.set_batch_mode(false);
    state.set_all_checked(false);
    state.set_all_indeterminate(false);
    state.set_checked_count(0);
    state.set_deleting(false);
    state.set_delete_modal_open(false);
    state.set_delete_modal_count(0);

    let weak_search = page.as_weak();
    state.on_search(move || {
        if let Some(page) = weak_search.upgrade() {
            on_filter_or_search_changed(&page);
        }
    });

    let weak_filter = page.as_weak();
    state.on_filter_changed(move || {
        if let Some(page) = weak_filter.upgrade() {
            on_filter_or_search_changed(&page);
        }
    });

    let weak_select = page.as_weak();
    state.on_select(move |id| {
        if let Some(page) = weak_select.upgrade() {
            select_entry(&page, i64::from(id));
        }
    });

    let weak_speak = page.as_weak();
    state.on_speak(move || {
        if let Some(page) = weak_speak.upgrade() {
            handle_speak(&page);
        }
    });

    let weak_export = page.as_weak();
    state.on_export_markdown(move || {
        if let Some(page) = weak_export.upgrade() {
            handle_export(&page);
        }
    });

    let weak_enter = page.as_weak();
    state.on_enter_batch_mode(move || {
        if let Some(page) = weak_enter.upgrade() {
            handle_enter_batch_mode(&page);
        }
    });

    let weak_finish = page.as_weak();
    state.on_finish_batch_mode(move || {
        if let Some(page) = weak_finish.upgrade() {
            handle_finish_batch_mode(&page);
        }
    });

    let weak_toggle_all = page.as_weak();
    state.on_toggle_all(move || {
        if let Some(page) = weak_toggle_all.upgrade() {
            handle_toggle_all(&page);
        }
    });

    let weak_toggle_row = page.as_weak();
    state.on_toggle_row(move |id, checked| {
        if let Some(page) = weak_toggle_row.upgrade() {
            handle_toggle_row(&page, i64::from(id), checked);
        }
    });

    let weak_del_single = page.as_weak();
    state.on_request_delete_single(move || {
        if let Some(page) = weak_del_single.upgrade() {
            handle_request_delete_single(&page);
        }
    });

    let weak_del_batch = page.as_weak();
    state.on_request_delete_batch(move || {
        if let Some(page) = weak_del_batch.upgrade() {
            handle_request_delete_batch(&page);
        }
    });

    let weak_confirm_del = page.as_weak();
    state.on_confirm_delete(move || {
        if let Some(page) = weak_confirm_del.upgrade() {
            handle_confirm_delete(&page);
        }
    });

    let weak_cancel_del = page.as_weak();
    state.on_cancel_delete(move || {
        if let Some(page) = weak_cancel_del.upgrade() {
            handle_cancel_delete(&page);
        }
    });

    let weak_changed = page.as_weak();
    wordbook::set_on_changed(Some(Box::new(move || {
        let weak = weak_changed.clone();
        // 回调在生词本线程上，切回 UI 线程
        // ignore: 窗口可能已关闭，丢弃该回调
        let _ = slint::invoke_from_event_loop(move || {
            let Some(page) = weak.upgrade() else { return };
            handle_wordbook_changed(&page);
        });
    })));

    load_list(page.as_weak());
}

fn on_filter_or_search_changed(page: &SettingsWindow) {
    // 勾选**不**跟着筛选/搜索清空：在「生词」里勾几条、切到「长难句」再勾几条，切回「全部」
    // 两边的勾都还在，可以一次删掉（用户手测提的）。
    // `checked` 一直是「全库范围的勾选集合」，「已选 N 项」和删除都只取它跟 `visible_ids` 的交集
    // （`checked_visible_count` / `freeze_pending`），所以当前页签上看到的数字和删掉的东西始终一致。
    // 界面上的全选框状态由 `apply_filter` 末尾的 `update_batch_selection_state` 按新的可见集重算。
    apply_filter(page);
}

fn handle_enter_batch_mode(page: &SettingsWindow) {
    let is_deleting = STATE.with_borrow(|s| s.deleting);
    if is_deleting {
        return;
    }
    STATE.with_borrow_mut(|s| {
        s.batch_mode = true;
        s.checked.clear();
    });
    let state = page.global::<WordbookState>();
    state.set_batch_mode(true);
    state.set_all_checked(false);
    state.set_all_indeterminate(false);
    state.set_checked_count(0);

    sync_rows_checked();
}

fn handle_finish_batch_mode(page: &SettingsWindow) {
    let is_deleting = STATE.with_borrow(|s| s.deleting);
    if is_deleting {
        return;
    }
    STATE.with_borrow_mut(|s| {
        s.batch_mode = false;
        s.checked.clear();
    });
    let state = page.global::<WordbookState>();
    state.set_batch_mode(false);
    state.set_all_checked(false);
    state.set_all_indeterminate(false);
    state.set_checked_count(0);

    sync_rows_checked();
}

fn handle_toggle_row(page: &SettingsWindow, id: i64, checked: bool) {
    let is_deleting = STATE.with_borrow(|s| s.deleting);
    if is_deleting {
        return;
    }
    STATE.with_borrow_mut(|s| {
        if checked {
            s.checked.insert(id);
        } else {
            s.checked.remove(&id);
        }
        if let Some(model) = &s.rows_model {
            for i in 0..model.row_count() {
                if let Some(mut row) = model.row_data(i)
                    && i64::from(row.id) == id
                {
                    row.checked = checked;
                    model.set_row_data(i, row);
                    break;
                }
            }
        }
    });
    update_batch_selection_state(page);
}

fn handle_toggle_all(page: &SettingsWindow) {
    let is_deleting = STATE.with_borrow(|s| s.deleting);
    if is_deleting {
        return;
    }
    STATE.with_borrow_mut(|s| {
        toggle_all(&mut s.checked, &s.visible_ids);
        sync_rows_checked_internal(s);
    });
    update_batch_selection_state(page);
}

fn sync_rows_checked() {
    STATE.with_borrow(sync_rows_checked_internal);
}

fn sync_rows_checked_internal(s: &WordbookUiState) {
    if let Some(model) = &s.rows_model {
        for i in 0..model.row_count() {
            if let Some(mut row) = model.row_data(i) {
                let is_checked = s.checked.contains(&i64::from(row.id));
                if row.checked != is_checked {
                    row.checked = is_checked;
                    model.set_row_data(i, row);
                }
            }
        }
    }
}

fn update_batch_selection_state(page: &SettingsWindow) {
    let (c_state, count) = STATE.with_borrow(|s| {
        let state = all_state(&s.checked, &s.visible_ids);
        let count = checked_visible_count(&s.checked, &s.visible_ids);
        (state, count)
    });
    let state = page.global::<WordbookState>();
    match c_state {
        CheckState::Checked => {
            state.set_all_checked(true);
            state.set_all_indeterminate(false);
        }
        CheckState::Unchecked => {
            state.set_all_checked(false);
            state.set_all_indeterminate(false);
        }
        CheckState::Indeterminate => {
            state.set_all_checked(false);
            state.set_all_indeterminate(true);
        }
    }
    state.set_checked_count(i32::try_from(count).unwrap_or(0));
}

fn handle_request_delete_single(page: &SettingsWindow) {
    let (sel_id, deleting) = STATE.with_borrow(|s| (s.selected_id, s.deleting));
    if deleting || sel_id == 0 {
        return;
    }
    STATE.with_borrow_mut(|s| {
        s.pending = vec![sel_id];
        s.pending_is_batch = false;
    });
    let state = page.global::<WordbookState>();
    state.set_delete_modal_count(1);
    state.set_delete_modal_open(true);
}

fn handle_request_delete_batch(page: &SettingsWindow) {
    let (pending, deleting) =
        STATE.with_borrow(|s| (freeze_pending(&s.checked, &s.visible_ids), s.deleting));
    if deleting || pending.is_empty() {
        return;
    }
    let count = pending.len();
    STATE.with_borrow_mut(|s| {
        s.pending = pending;
        s.pending_is_batch = true;
    });
    let state = page.global::<WordbookState>();
    state.set_delete_modal_count(i32::try_from(count).unwrap_or(0));
    state.set_delete_modal_open(true);
}

fn handle_cancel_delete(page: &SettingsWindow) {
    let is_deleting = STATE.with_borrow(|s| s.deleting);
    if is_deleting {
        return;
    }
    STATE.with_borrow_mut(|s| {
        s.pending.clear();
    });
    let state = page.global::<WordbookState>();
    state.set_delete_modal_open(false);
}

fn handle_confirm_delete(page: &SettingsWindow) {
    let (deleting, pending, current_id, visible_ids) = STATE.with_borrow(|s| {
        (
            s.deleting,
            s.pending.clone(),
            if s.selected_id != 0 {
                Some(s.selected_id)
            } else {
                None
            },
            s.visible_ids.clone(),
        )
    });
    if deleting || pending.is_empty() {
        return;
    }

    STATE.with_borrow_mut(|s| {
        s.deleting = true;
    });
    let state = page.global::<WordbookState>();
    state.set_deleting(true);

    // 删除后顺延：直接调 wordbook::next_selected
    let next_id = wordbook::next_selected(&visible_ids, &pending, current_id);

    let weak = page.as_weak();
    wordbook::delete(pending, move |res| {
        // ignore: 窗口可能已关闭，丢弃该回调
        let _ = slint::invoke_from_event_loop(move || {
            let Some(page) = weak.upgrade() else { return };
            STATE.with_borrow_mut(|s| {
                s.deleting = false;
            });
            match res {
                Ok(_) => {
                    let still_batch = STATE.with_borrow_mut(|s| {
                        let deleted = std::mem::take(&mut s.pending);
                        s.refresh_pending = false;
                        if s.pending_is_batch {
                            // 批量删完就退出多选（旧版 `finishBatch`）
                            s.batch_mode = false;
                            s.checked.clear();
                        } else {
                            // 多选模式下删的单条：只把它摘掉，其余勾选留着
                            s.checked.retain(|id| !deleted.contains(id));
                        }
                        s.batch_mode
                    });
                    let state = page.global::<WordbookState>();
                    state.set_deleting(false);
                    state.set_delete_modal_open(false);
                    state.set_batch_mode(still_batch);
                    if !still_batch {
                        state.set_all_checked(false);
                        state.set_all_indeterminate(false);
                        state.set_checked_count(0);
                    }

                    load_list_and_select(page.as_weak(), next_id);
                }
                Err(e) => {
                    log::warn!("Settings: wordbook delete failed: {e}");
                    let state = page.global::<WordbookState>();
                    state.set_deleting(false);
                    let msg = state.invoke_msg_delete_failed();
                    page.invoke_show_toast(msg, 2);

                    let replay = STATE.with_borrow_mut(|s| std::mem::take(&mut s.refresh_pending));
                    if replay {
                        load_list(page.as_weak());
                    }
                }
            }
        });
    });
}

fn handle_export(page: &SettingsWindow) {
    let dialog = rfd::FileDialog::new()
        .set_file_name(wordbook_export::default_file_name())
        .add_filter("Markdown", &["md"]);

    let Some(path) = dialog.save_file() else {
        return;
    };

    let weak = page.as_weak();
    wordbook::all(move |res| {
        let write_result = res.and_then(|entries| {
            let md = wordbook_export::build_markdown(&entries, chrono::Local::now());
            std::fs::write(&path, md).map_err(|e| crate::error::Error::Platform(e.to_string()))
        });

        let path_str = path.display().to_string();
        // ignore: 窗口可能已关闭，丢弃该回调
        let _ = slint::invoke_from_event_loop(move || {
            let Some(page) = weak.upgrade() else { return };
            let state = page.global::<WordbookState>();
            match write_result {
                Ok(()) => {
                    let msg = state.invoke_msg_export_ok(path_str.into());
                    page.invoke_show_toast(msg, 1);
                }
                Err(e) => {
                    log::warn!("Settings: export wordbook markdown failed: {e}");
                    let msg = state.invoke_msg_export_failed(e.to_string().into());
                    page.invoke_show_toast(msg, 2);
                }
            }
        });
    });
}

fn handle_wordbook_changed(page: &SettingsWindow) {
    let is_deleting = STATE.with_borrow(|s| s.deleting);
    if is_deleting {
        STATE.with_borrow_mut(|s| {
            s.refresh_pending = true;
        });
        return;
    }
    load_list(page.as_weak());
    // 列表那一路只在「选中项换了人」时才重查详情（`apply_filter_internal`），
    // 而晚到的 AI 结果是就地更新同一行：id 没变，右卡得自己再查一遍，否则停在早到的结果上。
    let selected = STATE.with_borrow(|s| s.selected_id);
    if selected != 0 {
        load_detail(page.as_weak(), selected);
    }
}

fn load_list(weak: slint::Weak<SettingsWindow>) {
    load_list_and_select(weak, None);
}

fn load_list_and_select(weak: slint::Weak<SettingsWindow>, target_id: Option<i64>) {
    wordbook::list(move |res| {
        // ignore: 窗口可能已关闭，丢弃该回调
        let _ = slint::invoke_from_event_loop(move || {
            let Some(page) = weak.upgrade() else { return };
            match res {
                Ok(summaries) => {
                    STATE.with_borrow_mut(|s| {
                        s.entries = summaries;
                    });
                    apply_filter_internal(&page, target_id);
                }
                Err(e) => {
                    log::warn!("Settings: load wordbook list failed: {e}");
                }
            }
        });
    });
}

fn apply_filter(page: &SettingsWindow) {
    apply_filter_internal(page, None);
}

fn apply_filter_internal(page: &SettingsWindow, target_id: Option<i64>) {
    let state = page.global::<WordbookState>();
    let keyword = state.get_keyword();
    let filter_idx = state.get_filter();

    let kind = match filter_idx {
        1 => Some(Kind::Word),
        2 => Some(Kind::Sentence),
        _ => None,
    };

    let (visible_summaries, is_batch_mode) = STATE.with_borrow(|s| {
        let summaries = wordbook::visible(&s.entries, keyword.as_str(), kind)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        (summaries, s.batch_mode)
    });

    let visible_ids: Vec<i64> = visible_summaries.iter().map(|s| s.id).collect();

    let rows: Vec<WordbookRow> = STATE.with_borrow(|s| {
        visible_summaries
            .into_iter()
            .map(|sum| WordbookRow {
                id: i32::try_from(sum.id).unwrap_or(0),
                text: sum.text.as_str().into(),
                preview: sum.preview.as_str().into(),
                checked: s.checked.contains(&sum.id),
            })
            .collect()
    });

    let rows_model = Rc::new(VecModel::from(rows));
    STATE.with_borrow_mut(|s| {
        s.visible_ids = visible_ids.clone();
        s.rows_model = Some(rows_model.clone());
    });

    state.set_rows(ModelRc::from(rows_model));

    let current_id = STATE.with_borrow(|s| {
        if s.selected_id != 0 {
            Some(s.selected_id)
        } else {
            None
        }
    });

    let next_id = match target_id {
        Some(target) => {
            if visible_ids.contains(&target) {
                Some(target)
            } else {
                fallback_selected(&visible_ids, current_id)
            }
        }
        None => fallback_selected(&visible_ids, current_id),
    };

    match next_id {
        Some(id) => {
            if current_id != Some(id) {
                select_entry(page, id);
            }
        }
        None => {
            select_entry(page, 0);
        }
    }

    if is_batch_mode {
        update_batch_selection_state(page);
    }
}

fn select_entry(page: &SettingsWindow, id: i64) {
    stop_speaking();
    let state = page.global::<WordbookState>();
    state.set_speaking(false);

    STATE.with_borrow_mut(|s| {
        s.selected_id = id;
    });
    state.set_selected_id(i32::try_from(id).unwrap_or(0));

    if id == 0 {
        state.set_detail_text("".into());
        state.set_detail_is_word(false);
        state.set_detail_category("".into());
        state.set_detail_difficulty(0);
        state.set_detail_difficulty_reason("".into());
        state.set_detail_entry(EntryView::default());
        return;
    }

    load_detail(page.as_weak(), id);
}

/// 只重查并重填右卡，不动选中项、不打断发音。选中项已经换人时丢弃这次结果。
fn load_detail(weak: slint::Weak<SettingsWindow>, id: i64) {
    wordbook::entry(id, move |res| {
        // ignore: 窗口可能已关闭，丢弃该回调
        let _ = slint::invoke_from_event_loop(move || {
            let Some(page) = weak.upgrade() else { return };
            let is_still_selected = STATE.with_borrow(|s| s.selected_id == id);
            if !is_still_selected {
                return;
            }
            match res {
                Ok(Some(entry)) => {
                    populate_detail(&page, &entry);
                }
                Ok(None) => {
                    log::warn!("Settings: wordbook entry {id} not found");
                }
                Err(e) => {
                    log::warn!("Settings: load wordbook entry {id} failed: {e}");
                }
            }
        });
    });
}

fn populate_detail(page: &SettingsWindow, entry: &Entry) {
    let display = result::entry_display(entry.detail.as_ref(), &entry.translation);
    let is_word = entry.kind == Kind::Word && display.kind != DisplayKind::Sentence;
    let category = format_category(display.category.as_deref());
    let difficulty = display.difficulty.map(i32::from).unwrap_or(0);
    let difficulty_reason = display.difficulty_reason.clone();
    let detail_entry = crate::ui::entry_view::to_view(&display, &entry.translation);

    let state = page.global::<WordbookState>();
    state.set_detail_text(entry.text.as_str().into());
    state.set_detail_is_word(is_word);
    state.set_detail_category(category.into());
    state.set_detail_difficulty(difficulty);
    state.set_detail_difficulty_reason(difficulty_reason.into());
    state.set_detail_entry(detail_entry);
}

fn handle_speak(page: &SettingsWindow) {
    let state = page.global::<WordbookState>();
    let is_speaking = STATE.with_borrow(|s| s.speaking);
    if is_speaking {
        stop_speaking();
        state.set_speaking(false);
        return;
    }

    let text = state.get_detail_text().to_string();
    if text.is_empty() {
        return;
    }

    let next_token = STATE.with_borrow_mut(|s| {
        s.speak_token = s.speak_token.wrapping_add(1);
        s.speaking = true;
        s.speak_token
    });
    state.set_speaking(true);

    let weak = page.as_weak();
    let voice = result::voice_for(&text);
    let res = platform::speak(&text, voice, move || {
        // ignore: 窗口可能已关闭，丢弃完成回调
        let _ = slint::invoke_from_event_loop(move || {
            let matches = STATE.with_borrow(|s| s.speak_token == next_token);
            if matches {
                STATE.with_borrow_mut(|s| s.speaking = false);
                if let Some(page) = weak.upgrade() {
                    page.global::<WordbookState>().set_speaking(false);
                }
            }
        });
    });

    if let Err(e) = res {
        log::warn!("Settings: wordbook speak failed: {e}");
        STATE.with_borrow_mut(|s| s.speaking = false);
        state.set_speaking(false);
    }
}

pub fn stop_speaking() {
    platform::stop_speaking();
    STATE.with_borrow_mut(|s| {
        s.speak_token = s.speak_token.wrapping_add(1);
        s.speaking = false;
    });
}

pub fn on_close() {
    wordbook::set_on_changed(None);
    stop_speaking();
    STATE.with_borrow_mut(|s| {
        s.entries.clear();
        s.selected_id = 0;
        s.batch_mode = false;
        s.checked.clear();
        s.deleting = false;
        s.pending.clear();
        s.pending_is_batch = false;
        s.refresh_pending = false;
        s.visible_ids.clear();
        s.rows_model = None;
    });
}

/// 将分类码转换为展示字符串，如 "EN01" -> "英文 · 01 定语从句类"。
/// 无分类或未知分类码返回空串。
pub fn format_category(code: Option<&str>) -> String {
    format_category_for(code, &crate::logic::config::snapshot().general.language)
}

/// 按指定语言代码将分类码转换为展示字符串。
pub fn format_category_for(code: Option<&str>, lang: &str) -> String {
    let Some(code) = code else {
        return String::new();
    };
    match result::category(code) {
        Some(cat) => format!("{} · {} {}", cat.lang_for(lang), cat.no, cat.name_for(lang)),
        None => String::new(),
    }
}

/// 决定可见结果变化后的选中项：
/// - 若可见列表为空，返回 None（清空详情）。
/// - 若之前选中的 id 仍在当前可见列表中，保持它。
/// - 否则（之前未选或当前已不可见），退回当前可见列表的第一条（旧版 `?? list[0]`）。
pub fn fallback_selected(visible_ids: &[i64], current: Option<i64>) -> Option<i64> {
    if visible_ids.is_empty() {
        return None;
    }
    if let Some(id) = current
        && visible_ids.contains(&id)
    {
        return Some(id);
    }
    visible_ids.first().copied()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Unchecked,
    Checked,
    Indeterminate,
}

/// 计算全选复选框的三态：全部勾选 / 未勾选 / 半选
pub fn all_state(checked: &HashSet<i64>, visible: &[i64]) -> CheckState {
    if visible.is_empty() {
        return CheckState::Unchecked;
    }
    let count = checked_visible_count(checked, visible);
    if count == visible.len() {
        CheckState::Checked
    } else if count == 0 {
        CheckState::Unchecked
    } else {
        CheckState::Indeterminate
    }
}

/// 点击全选复选框：全勾时取消全部，否则勾上全部当前可见项
pub fn toggle_all(checked: &mut HashSet<i64>, visible: &[i64]) {
    let state = all_state(checked, visible);
    if state == CheckState::Checked {
        for id in visible {
            checked.remove(id);
        }
    } else {
        for id in visible {
            checked.insert(*id);
        }
    }
}

/// 计算当前可见项中已被勾选的项数
pub fn checked_visible_count(checked: &HashSet<i64>, visible: &[i64]) -> usize {
    visible.iter().filter(|id| checked.contains(id)).count()
}

/// 冻结要删除的 ID 集合（仅包含当前可见且勾选的项，保证去重与快照隔离）
pub fn freeze_pending(checked: &HashSet<i64>, visible: &[i64]) -> Vec<i64> {
    visible
        .iter()
        .filter(|id| checked.contains(id))
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_category_known_and_unknown() {
        assert_eq!(format_category(Some("EN01")), "英文 · 01 定语从句类");
        assert_eq!(format_category(Some("ZH03")), "中文 · 03 多层复句嵌套类");
        assert_eq!(format_category(Some("ZH05")), "中文 · 05 长连谓与兼语句");
        assert_eq!(format_category(Some("UNKNOWN")), "");
        assert_eq!(format_category(None), "");
        assert_eq!(
            format_category_for(Some("EN01"), "en"),
            "English · 01 Relative clauses"
        );
        assert_eq!(
            format_category_for(Some("ZH03"), "en"),
            "Chinese · 03 Nested complex sentences"
        );
        assert_eq!(
            format_category_for(Some("ZH05"), "en"),
            "Chinese · 05 Serial verbs & pivotal sentences"
        );
    }

    #[test]
    fn fallback_selected_cases() {
        // 1. 空列表
        assert_eq!(fallback_selected(&[], None), None);
        assert_eq!(fallback_selected(&[], Some(42)), None);

        // 2. 之前未选，退回首条
        assert_eq!(fallback_selected(&[10, 20, 30], None), Some(10));

        // 3. 之前选中的仍在列表中，保持
        assert_eq!(fallback_selected(&[10, 20, 30], Some(20)), Some(20));

        // 4. 之前选中的已不在当前列表中，退回首条
        assert_eq!(fallback_selected(&[10, 20, 30], Some(99)), Some(10));
    }

    #[test]
    fn multi_select_state_machine() {
        let visible = vec![1, 2, 3];
        let mut checked = HashSet::new();

        // 1. 初始未勾选
        assert_eq!(all_state(&checked, &visible), CheckState::Unchecked);
        assert_eq!(checked_visible_count(&checked, &visible), 0);

        // 2. 勾选部分：半选状态
        checked.insert(1);
        assert_eq!(all_state(&checked, &visible), CheckState::Indeterminate);
        assert_eq!(checked_visible_count(&checked, &visible), 1);

        // 3. 点击全选：变为全部勾选
        toggle_all(&mut checked, &visible);
        assert_eq!(all_state(&checked, &visible), CheckState::Checked);
        assert_eq!(checked_visible_count(&checked, &visible), 3);

        // 4. 再次点击全选：取消全部
        toggle_all(&mut checked, &visible);
        assert_eq!(all_state(&checked, &visible), CheckState::Unchecked);
        assert_eq!(checked_visible_count(&checked, &visible), 0);

        // 5. 空可见列表一律 Unchecked
        assert_eq!(all_state(&checked, &[]), CheckState::Unchecked);

        // 6. 勾选跨筛选保留：集合里留着不在当前可见集合里的 id，计数和全选态只看交集
        checked.insert(2);
        checked.insert(999); // 切到别的页签才看得见的那一条
        assert_eq!(checked_visible_count(&checked, &visible), 1);
        assert_eq!(all_state(&checked, &visible), CheckState::Indeterminate);
        assert_eq!(
            freeze_pending(&checked, &visible),
            vec![2],
            "删除只动当前可见的那些，999 留着"
        );
        checked.clear();
        assert_eq!(all_state(&checked, &visible), CheckState::Unchecked);

        // 7. 确认删除时冻结快照，新到达的条目不进入待删除集合
        checked.insert(1);
        checked.insert(2);
        let pending = freeze_pending(&checked, &visible);
        assert_eq!(pending, vec![1, 2]);

        // 新条目 4 插入到可见列表首位
        let visible_with_new = vec![4, 1, 2, 3];
        // pending 集合保持不变，条目 4 不在其中
        assert_eq!(pending, vec![1, 2]);
        assert!(!pending.contains(&4));
        assert_eq!(freeze_pending(&checked, &visible_with_new), vec![1, 2]);
    }

    /// 批量删除的两个纯计算：冻结的快照、顺延落点。
    ///
    /// 删除**失败**那条路没有独立测试：它的行为是「什么都不做」——不清 `pending`、
    /// 不清 `checked`、不关确认框，只弹一句 toast（见 `handle_confirm_delete` 的 `Err` 支）。
    /// 没有分支可断言，写出来的只会是自己断言自己。失败重试放进手测清单。
    #[test]
    fn batch_delete_freezes_snapshot_and_skips_removed() {
        let visible = vec![10, 20, 30, 40];
        let mut checked = HashSet::new();
        checked.insert(20);
        checked.insert(30);

        let pending = freeze_pending(&checked, &visible);
        assert_eq!(pending, vec![20, 30], "按可见顺序冻结，不是 HashSet 的乱序");

        // 选中项落在删除集合里：跳过整段连续的删除项，落到 40
        assert_eq!(
            wordbook::next_selected(&visible, &pending, Some(20)),
            Some(40)
        );
        assert_eq!(
            wordbook::next_selected(&visible, &pending, Some(30)),
            Some(40)
        );
        // 选中项不在删除集合里：不动
        assert_eq!(
            wordbook::next_selected(&visible, &pending, Some(10)),
            Some(10)
        );
        // 删到末尾：往上找
        let tail = freeze_pending(&HashSet::from([30, 40]), &visible);
        assert_eq!(wordbook::next_selected(&visible, &tail, Some(40)), Some(20));

        // 快照已冻结：确认框开着的时候再勾中新条目也不进这一批
        checked.insert(40);
        assert_eq!(pending, vec![20, 30]);
    }
}
