//! 将 logic 层的 EntryDisplay 展示投影转换为 Slint 界面专用的 EntryView 结构体。

use slint::{ModelRc, SharedString, VecModel};

use crate::logic::result::EntryDisplay;
use crate::slint_ui::{EntryExample, EntryExplain, EntryView, EntryVocab};

/// 将展示投影转换为 Slint 视图结构体。
///
/// 当 `display` 全空而 `fallback_text` 不空时，使用 `fallback_text` 填充 `translation`，
/// 保证纯文本结果有内容展示。
pub fn to_view(display: &EntryDisplay, fallback_text: &str) -> EntryView {
    let all_empty = is_all_empty(display);
    let translation: SharedString = if all_empty && !fallback_text.is_empty() {
        fallback_text.into()
    } else {
        display.translation.as_str().into()
    };

    let explanations: Vec<EntryExplain> = display
        .explanations
        .iter()
        .map(|e| EntryExplain {
            part: e.part.as_str().into(),
            text: e.explains.join(", ").into(),
        })
        .collect();

    let examples: Vec<EntryExample> = display
        .examples
        .iter()
        .map(|e| EntryExample {
            text: e.text.as_str().into(),
            translation: e.translation.as_str().into(),
        })
        .collect();

    let notes: Vec<SharedString> = display.notes.iter().map(|n| n.as_str().into()).collect();

    let (main_clause, clauses) = match &display.syntax_breakdown {
        Some(s) => (
            s.main_clause.as_str().into(),
            s.clauses_and_modifiers.as_str().into(),
        ),
        None => (SharedString::default(), SharedString::default()),
    };

    let vocabulary: Vec<EntryVocab> = display
        .key_vocabulary
        .iter()
        .map(|v| EntryVocab {
            word: v.word.as_str().into(),
            meaning: v.meaning_in_context.as_str().into(),
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
    }
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
}
