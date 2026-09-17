//! 设置窗口：生词本页（列表、搜索、筛选、右卡详情、发音）。

use slint::{ComponentHandle, ModelRc, VecModel};
use std::cell::RefCell;

use crate::logic::result::{self, DisplayKind, Kind};
use crate::logic::wordbook::{self, Entry, Summary};
use crate::platform;
use crate::slint_ui::{EntryView, SettingsWindow, WordbookRow, WordbookState};

struct WordbookUiState {
    entries: Vec<Summary>,
    selected_id: i64,
    speak_token: u64,
    speaking: bool,
}

thread_local! {
    static STATE: RefCell<WordbookUiState> = const { RefCell::new(WordbookUiState {
        entries: Vec::new(),
        selected_id: 0,
        speak_token: 0,
        speaking: false,
    }) };
}

pub fn bind(page: &SettingsWindow) {
    STATE.with_borrow_mut(|s| {
        s.entries.clear();
        s.selected_id = 0;
        s.speak_token = s.speak_token.wrapping_add(1);
        s.speaking = false;
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

    let weak_search = page.as_weak();
    state.on_search(move || {
        if let Some(page) = weak_search.upgrade() {
            apply_filter(&page);
        }
    });

    let weak_filter = page.as_weak();
    state.on_filter_changed(move || {
        if let Some(page) = weak_filter.upgrade() {
            apply_filter(&page);
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

    load_list(page.as_weak());
}

fn load_list(weak: slint::Weak<SettingsWindow>) {
    wordbook::list(move |res| {
        // ignore: 窗口可能已关闭，丢弃该回调
        let _ = slint::invoke_from_event_loop(move || {
            let Some(page) = weak.upgrade() else { return };
            match res {
                Ok(summaries) => {
                    STATE.with_borrow_mut(|s| {
                        s.entries = summaries;
                    });
                    apply_filter(&page);
                }
                Err(e) => {
                    log::warn!("Settings: load wordbook list failed: {e}");
                }
            }
        });
    });
}

fn apply_filter(page: &SettingsWindow) {
    let state = page.global::<WordbookState>();
    let keyword = state.get_keyword();
    let filter_idx = state.get_filter();

    let kind = match filter_idx {
        1 => Some(Kind::Word),
        2 => Some(Kind::Sentence),
        _ => None,
    };

    let visible_summaries = STATE.with_borrow(|s| {
        wordbook::visible(&s.entries, keyword.as_str(), kind)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>()
    });

    let visible_ids: Vec<i64> = visible_summaries.iter().map(|s| s.id).collect();

    let rows: Vec<WordbookRow> = visible_summaries
        .into_iter()
        .map(|s| WordbookRow {
            id: i32::try_from(s.id).unwrap_or(0),
            text: s.text.as_str().into(),
            preview: s.preview.as_str().into(),
        })
        .collect();

    state.set_rows(ModelRc::new(VecModel::from(rows)));

    let current_id = STATE.with_borrow(|s| {
        if s.selected_id != 0 {
            Some(s.selected_id)
        } else {
            None
        }
    });
    let next_id = fallback_selected(&visible_ids, current_id);

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

    let weak = page.as_weak();
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
    stop_speaking();
    STATE.with_borrow_mut(|s| {
        s.entries.clear();
        s.selected_id = 0;
    });
}

/// 将分类码转换为展示字符串，如 "EN01" -> "英文 · 01 定语从句类"。
/// 无分类或未知分类码返回空串。
pub fn format_category(code: Option<&str>) -> String {
    let Some(code) = code else {
        return String::new();
    };
    match result::category(code) {
        Some(cat) => format!("{} · {} {}", cat.lang, cat.no, cat.name),
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
}
