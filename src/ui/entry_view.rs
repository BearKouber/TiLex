//! 将 logic 层的 EntryDisplay 展示投影转换为 Slint 界面专用的 EntryView 结构体。

use slint::{ModelRc, SharedString, VecModel};

use crate::logic::result::EntryDisplay;
use crate::slint_ui::{EntryExample, EntryExplain, EntryView, EntryVocab};

/// 将展示投影转换为 Slint 视图结构体（无高亮模式，用于浮窗划词结果展示）。
///
/// 当 `display` 全空而 `fallback_text` 不空时，使用 `fallback_text` 填充 `translation`，
/// 保证纯文本结果有内容展示。
pub fn to_view(display: &EntryDisplay, fallback_text: &str) -> EntryView {
    to_view_internal(display, fallback_text, None)
}

/// 将展示投影转换为带中英分色与关键词高亮的 Slint 视图结构体（用于生词本详情展示）。
pub fn to_view_highlighted(
    display: &EntryDisplay,
    fallback_text: &str,
    keyword: &str,
    accent_color: &str,
    target_color: &str,
) -> EntryView {
    to_view_internal(
        display,
        fallback_text,
        Some(StyleParams {
            keyword,
            accent_color,
            target_color,
        }),
    )
}

struct StyleParams<'a> {
    keyword: &'a str,
    accent_color: &'a str,
    target_color: &'a str,
}

fn to_view_internal(
    display: &EntryDisplay,
    fallback_text: &str,
    style: Option<StyleParams<'_>>,
) -> EntryView {
    let all_empty = is_all_empty(display);
    let has_highlight = style.is_some();

    let raw_translation = if all_empty && !fallback_text.is_empty() {
        fallback_text
    } else {
        display.translation.as_str()
    };
    let translation: SharedString = raw_translation.into();
    let styled_translation = if let Some(ref s) = style {
        styled(raw_translation, s, Role::Target)
    } else {
        slint::StyledText::default()
    };

    let explanations: Vec<EntryExplain> = display
        .explanations
        .iter()
        .map(|e| {
            let joined = e.explains.join(", ");
            let styled_text = if let Some(ref s) = style {
                styled(&joined, s, Role::Target)
            } else {
                slint::StyledText::default()
            };
            EntryExplain {
                part: e.part.as_str().into(),
                text: joined.into(),
                styled_text,
            }
        })
        .collect();

    let styled_associations = if let Some(ref s) = style {
        if display.associations.is_empty() {
            slint::StyledText::default()
        } else {
            let joined = display.associations.join(", ");
            styled(&joined, s, Role::Source)
        }
    } else {
        slint::StyledText::default()
    };

    let examples: Vec<EntryExample> = display
        .examples
        .iter()
        .map(|e| {
            let (styled_text, styled_translation) = if let Some(ref s) = style {
                (
                    styled(&e.text, s, Role::Source),
                    styled(&e.translation, s, Role::Target),
                )
            } else {
                (slint::StyledText::default(), slint::StyledText::default())
            };
            EntryExample {
                text: e.text.as_str().into(),
                translation: e.translation.as_str().into(),
                styled_text,
                styled_translation,
            }
        })
        .collect();

    let notes: Vec<SharedString> = display.notes.iter().map(|n| n.as_str().into()).collect();
    let styled_notes: Vec<slint::StyledText> = if let Some(ref s) = style {
        display
            .notes
            .iter()
            .map(|n| styled(n, s, Role::Target))
            .collect()
    } else {
        Vec::new()
    };

    let (main_clause, clauses, styled_main_clause, styled_clauses) = match &display.syntax_breakdown
    {
        Some(sb) => {
            let (sm, sc) = if let Some(ref s) = style {
                (
                    styled(&sb.main_clause, s, Role::Source),
                    styled(&sb.clauses_and_modifiers, s, Role::Clauses),
                )
            } else {
                (slint::StyledText::default(), slint::StyledText::default())
            };
            (
                sb.main_clause.as_str().into(),
                sb.clauses_and_modifiers.as_str().into(),
                sm,
                sc,
            )
        }
        None => (
            SharedString::default(),
            SharedString::default(),
            slint::StyledText::default(),
            slint::StyledText::default(),
        ),
    };

    let styled_nuance = if let Some(ref s) = style {
        if display.nuance_note.is_empty() {
            slint::StyledText::default()
        } else {
            styled(&display.nuance_note, s, Role::Target)
        }
    } else {
        slint::StyledText::default()
    };

    let vocabulary: Vec<EntryVocab> = display
        .key_vocabulary
        .iter()
        .map(|v| {
            let (styled_word, styled_meaning) = if let Some(ref s) = style {
                (
                    styled(&v.word, s, Role::Source),
                    styled(&v.meaning_in_context, s, Role::Target),
                )
            } else {
                (slint::StyledText::default(), slint::StyledText::default())
            };
            EntryVocab {
                word: v.word.as_str().into(),
                meaning: v.meaning_in_context.as_str().into(),
                styled_word,
                styled_meaning,
            }
        })
        .collect();

    EntryView {
        pronunciations: display.pronunciations.join("  ").into(),
        translation,
        explanations: ModelRc::new(VecModel::from(explanations)),
        associations: display.associations.join(", ").into(),
        examples: ModelRc::new(VecModel::from(examples)),
        notes: ModelRc::new(VecModel::from(notes)),
        main_clause,
        clauses,
        nuance: display.nuance_note.as_str().into(),
        vocabulary: ModelRc::new(VecModel::from(vocabulary)),
        has_highlight,
        styled_translation,
        styled_associations,
        styled_notes: ModelRc::new(VecModel::from(styled_notes)),
        styled_main_clause,
        styled_clauses,
        styled_nuance,
    }
}

/// 生词本详情里一段文字属于原文还是译文，决定它是黑还是灰（用户定：原文黑、译文灰，按角色不按文字）。
#[derive(Clone, Copy)]
enum Role {
    Source,
    Target,
    /// 修饰成分「原文片段 [语法作用]」：片段是原文，方括号里是译文语言
    Clauses,
}

fn styled(text: &str, s: &StyleParams<'_>, role: Role) -> slint::StyledText {
    if text.is_empty() {
        return slint::StyledText::default();
    }
    let tinted = match role {
        Role::Source => Vec::new(),
        Role::Target => std::iter::once(0..text.len()).collect(),
        Role::Clauses => crate::logic::wordbook::bracket_ranges(text),
    };
    let md = crate::logic::wordbook::tinted_markdown(
        text,
        s.keyword,
        s.accent_color,
        s.target_color,
        &tinted,
    );
    slint::StyledText::from_markdown(&md)
        .unwrap_or_else(|_| slint::StyledText::from_plain_text(text))
}

/// 辅助函数：将纯文本或高亮 Markdown 转换为 Slint 的 StyledText。
/// 当 Markdown 解析失败时优雅降级为纯文本 StyledText。
pub fn to_styled_text(text: &str, keyword: &str, accent_color: &str) -> slint::StyledText {
    if keyword.trim().is_empty() || text.is_empty() {
        return slint::StyledText::from_plain_text(text);
    }
    let md = crate::logic::wordbook::highlight_markdown(text, keyword, accent_color);
    slint::StyledText::from_markdown(&md)
        .unwrap_or_else(|_| slint::StyledText::from_plain_text(text))
}

fn is_all_empty(display: &EntryDisplay) -> bool {
    let syntax_empty = match &display.syntax_breakdown {
        None => true,
        Some(s) => s.main_clause.is_empty() && s.clauses_and_modifiers.is_empty(),
    };

    display.translation.is_empty()
        && display.pronunciations.is_empty()
        && display.explanations.is_empty()
        && display.associations.is_empty()
        && display.examples.is_empty()
        && display.notes.is_empty()
        && syntax_empty
        && display.nuance_note.is_empty()
        && display.key_vocabulary.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    use crate::logic::result::{DisplayKind, Example, Explanation, Syntax, Vocab};

    fn empty_display(kind: DisplayKind) -> EntryDisplay {
        EntryDisplay {
            kind,
            translation: String::new(),
            pronunciations: Vec::new(),
            explanations: Vec::new(),
            associations: Vec::new(),
            examples: Vec::new(),
            notes: Vec::new(),
            category: None,
            difficulty: None,
            difficulty_reason: String::new(),
            syntax_breakdown: None,
            nuance_note: String::new(),
            key_vocabulary: Vec::new(),
        }
    }

    #[test]
    fn test_to_view_word() {
        let mut display = empty_display(DisplayKind::Word);
        display.pronunciations = vec!["/ˈæp.əl/".into(), "/'æp.l/".into()];
        display.explanations = vec![
            Explanation {
                part: "n.".into(),
                explains: vec!["苹果".into(), "苹果树".into()],
            },
            Explanation {
                part: "vt.".into(),
                explains: vec!["使成苹果形".into()],
            },
        ];
        display.associations = vec!["pear".into(), "banana".into()];
        display.examples = vec![
            Example {
                text: "He ate an apple.".into(),
                translation: "他吃了一个苹果。".into(),
            },
            Example {
                text: "An apple a day keeps the doctor away.".into(),
                translation: "一天一苹果，医生远离我。".into(),
            },
        ];

        let view = to_view(&display, "");

        assert_eq!(view.pronunciations, "/ˈæp.əl/  /'æp.l/");
        assert_eq!(view.translation, "");
        assert_eq!(view.associations, "pear, banana");

        assert_eq!(view.explanations.row_count(), 2);
        let exp0 = view.explanations.row_data(0).unwrap();
        assert_eq!(exp0.part, "n.");
        assert_eq!(exp0.text, "苹果, 苹果树");
        let exp1 = view.explanations.row_data(1).unwrap();
        assert_eq!(exp1.part, "vt.");
        assert_eq!(exp1.text, "使成苹果形");

        assert_eq!(view.examples.row_count(), 2);
        let ex0 = view.examples.row_data(0).unwrap();
        assert_eq!(ex0.text, "He ate an apple.");
        assert_eq!(ex0.translation, "他吃了一个苹果。");
        let ex1 = view.examples.row_data(1).unwrap();
        assert_eq!(ex1.text, "An apple a day keeps the doctor away.");
        assert_eq!(ex1.translation, "一天一苹果，医生远离我。");

        assert_eq!(view.notes.row_count(), 0);
        assert_eq!(view.main_clause, "");
        assert_eq!(view.clauses, "");
        assert_eq!(view.nuance, "");
        assert_eq!(view.vocabulary.row_count(), 0);
    }

    #[test]
    fn test_to_view_sentence() {
        let mut display = empty_display(DisplayKind::Sentence);
        display.translation = "尽管天下着雨，他们依然继续前行。".into();
        display.syntax_breakdown = Some(Syntax {
            main_clause: "They kept walking".into(),
            clauses_and_modifiers: "although it was raining heavily".into(),
        });
        display.nuance_note = "书面语，表转折与坚韧。".into();
        display.key_vocabulary = vec![
            Vocab {
                word: "kept".into(),
                meaning_in_context: "保持，继续".into(),
            },
            Vocab {
                word: "heavily".into(),
                meaning_in_context: "沉重地，猛烈地".into(),
            },
        ];

        let view = to_view(&display, "fallback");

        // 非全空时使用 display.translation
        assert_eq!(view.translation, "尽管天下着雨，他们依然继续前行。");
        assert_eq!(view.main_clause, "They kept walking");
        assert_eq!(view.clauses, "although it was raining heavily");
        assert_eq!(view.nuance, "书面语，表转折与坚韧。");

        assert_eq!(view.vocabulary.row_count(), 2);
        let v0 = view.vocabulary.row_data(0).unwrap();
        assert_eq!(v0.word, "kept");
        assert_eq!(v0.meaning, "保持，继续");
        let v1 = view.vocabulary.row_data(1).unwrap();
        assert_eq!(v1.word, "heavily");
        assert_eq!(v1.meaning, "沉重地，猛烈地");
    }

    #[test]
    fn test_to_view_empty_display_fallback() {
        let display = empty_display(DisplayKind::Text);
        let view = to_view(&display, "fallback translation text");

        assert_eq!(view.translation, "fallback translation text");
        assert_eq!(view.pronunciations, "");
        assert_eq!(view.explanations.row_count(), 0);
        assert_eq!(view.associations, "");
        assert_eq!(view.examples.row_count(), 0);
        assert_eq!(view.notes.row_count(), 0);
        assert_eq!(view.main_clause, "");
        assert_eq!(view.clauses, "");
        assert_eq!(view.nuance, "");
        assert_eq!(view.vocabulary.row_count(), 0);
    }

    #[test]
    fn test_to_view_highlighted() {
        let mut display = empty_display(DisplayKind::Sentence);
        display.translation = "这是测试译文".into();
        display.notes = vec!["测试用法与语法".into()];
        let view = to_view_highlighted(&display, "", "测试", "#3b82f6", "#71717a");

        assert!(view.has_highlight);
        assert_eq!(view.translation, "这是测试译文");
        assert_eq!(view.notes.row_count(), 1);
        assert_eq!(view.styled_notes.row_count(), 1);

        // 生词本详情不管有无关键词，都走 StyledText 渲染（has_highlight 为 true）
        let view_no_kw = to_view_highlighted(&display, "", "", "", "#71717a");
        assert!(view_no_kw.has_highlight);

        // 浮窗用的 to_view 保持 has_highlight 为 false、styled 字段为空
        let view_plain = to_view(&display, "");
        assert!(!view_plain.has_highlight);
        assert_eq!(view_plain.styled_notes.row_count(), 0);
    }
}
