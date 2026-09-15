//! 收藏（从旧 `saved_entry.js` 移植）：一次划词 = 一条生词本记录。
//! 用户点收藏后，这次划词后面到的结果继续更新同一条（不管面板是否已经换了新词）——
//! 这是和面板 `query_id` 分开的另一种生命周期（design §2.4）。
//! 写库的顺序由生词本线程的 FIFO 保证；这里只算「该存成什么」。

use serde_json::Value;

use crate::logic::result::{self, Kind};

/// 一行服务的当前状态。顺序是请求开始时冻结的服务顺序。
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// 服务实例 id（写进生词本的 `service` 列）。
    pub service_id: String,
    /// 来源由请求决定（配置里是 AI 服务），**不信模型自己声明的**。
    pub is_ai: bool,
    /// `None` = 还在等 / 失败了。
    pub result: Option<Value>,
}

/// 要写进生词本的一条。
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub text: String,
    /// 完整纯文本（搜索、列表用）。
    pub translation: String,
    /// 规范化后的结构；纯文本结果是 `None`。
    pub detail: Option<Value>,
    /// 选中的那个结果来自哪个服务实例；都没结果时为空。
    pub service: String,
}

struct Candidate {
    translation: String,
    detail: Option<Value>,
    priority: u8,
}

fn candidate(item: &Item) -> Option<Candidate> {
    let value = item.result.as_ref()?;
    let detail = result::normalize_result(value, None);
    let translation = result::result_text(value);
    if translation.is_empty() {
        return None;
    }
    // 句子只有解析或标记也算「有补充」，否则排在前面的谷歌纯译文会打平胜出，标记就丢了。
    let supplemented_ai = item.is_ai
        && detail.as_ref().is_some_and(|d| {
            d["schemaVersion"] == 1
                && (d["examples"].as_array().is_some_and(|a| !a.is_empty())
                    || d["notes"].as_array().is_some_and(|a| !a.is_empty())
                    || [
                        "syntax_breakdown",
                        "nuance_note",
                        "key_vocabulary",
                        "category",
                        "difficulty",
                    ]
                    .iter()
                    .any(|k| d.get(*k).is_some()))
        });
    let dictionary = detail.as_ref().is_some_and(|d| d["kind"] != "sentence");
    let priority = if supplemented_ai {
        0
    } else if dictionary {
        1
    } else {
        2
    };
    Some(Candidate {
        translation,
        detail,
        priority,
    })
}

/// 从本轮可用结果里选**一个**完整结果（不拼接不同服务）：有补充的新 AI 结构 > 合法词典 > 其他文本；
/// 同级按服务顺序，与完成先后无关。`target` 是用户点了哪一行的星，那一行有结果就用它。
pub fn entry_snapshot(text: &str, items: &[Item], target: Option<usize>) -> Snapshot {
    let targeted = target
        .and_then(|i| items.get(i))
        .and_then(|item| candidate(item).map(|c| (c, item)));
    let selected = targeted.or_else(|| {
        let mut best: Option<(Candidate, &Item)> = None;
        for item in items {
            if let Some(c) = candidate(item)
                && best.as_ref().is_none_or(|(b, _)| c.priority < b.priority)
            {
                best = Some((c, item));
            }
        }
        best
    });
    match selected {
        Some((c, item)) => Snapshot {
            text: text.to_owned(),
            translation: c.translation,
            detail: c.detail,
            service: item.service_id.clone(),
        },
        None => Snapshot {
            text: text.to_owned(),
            translation: String::new(),
            detail: None,
            service: String::new(),
        },
    }
}

/// 一次划词的收藏状态。收藏前只记结果不写库；收藏后每次变化都返回一份要写的快照。
pub struct SavedEntry {
    text: String,
    items: Vec<Item>,
    requested: bool,
    target: Option<usize>,
}

impl SavedEntry {
    pub fn new(text: &str, items: Vec<Item>) -> Self {
        Self {
            text: text.to_owned(),
            items,
            requested: false,
            target: None,
        }
    }

    pub fn requested(&self) -> bool {
        self.requested
    }

    pub fn kind(&self) -> Kind {
        Kind::of(&self.text)
    }

    /// 第 `row` 个服务出结果（`None` = 失败）。已收藏时返回要写库的快照。
    pub fn patch(&mut self, row: usize, result: Option<Value>) -> Option<Snapshot> {
        if let Some(item) = self.items.get_mut(row) {
            item.result = result;
        }
        self.requested.then(|| self.snapshot())
    }

    /// 用户点收藏（`row` = 点的哪一行，顶栏的收藏传 `None`）。返回要写库的快照。
    pub fn save(&mut self, row: Option<usize>) -> Snapshot {
        if row.is_some() {
            self.target = row;
        }
        self.requested = true;
        self.snapshot()
    }

    fn snapshot(&self) -> Snapshot {
        entry_snapshot(&self.text, &self.items, self.target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dictionary() -> Value {
        json!({
            "pronunciations": [{"symbol": "/a/"}, {"symbol": "/b/"}],
            "explanations": [{"trait": "n.", "explains": ["translation"]}],
            "associations": ["machine translation", "literal translation"],
        })
    }
    fn sentence() -> Value {
        json!({
            "schemaVersion": 1, "kind": "sentence", "translation": "完整译文。",
            "examples": [{"text": "Literal example.", "translation": "例句译文。"}],
            "notes": ["Usage note.", "Grammar note."],
        })
    }
    fn item(id: &str, is_ai: bool, result: Option<Value>) -> Item {
        Item {
            service_id: id.into(),
            is_ai,
            result,
        }
    }
    fn google(v: Value) -> Item {
        item("google", false, Some(v))
    }
    fn ai(v: Value) -> Item {
        item("ai@one", true, Some(v))
    }

    #[test]
    fn saves_keep_the_whole_structure() {
        let s = entry_snapshot("translation", &[google(dictionary())], None);
        assert_eq!(s.detail, Some(dictionary()));
        assert_eq!(s.service, "google");
        let s = entry_snapshot("source", &[ai(sentence())], None);
        assert_eq!(s.detail, Some(sentence()));
        assert_eq!(s.translation, result::result_text(&sentence()));
        let bare = entry_snapshot(
            "source",
            &[ai(
                json!({"kind": "sentence", "translation": "translation"}),
            )],
            None,
        );
        assert_eq!(
            bare.detail,
            Some(
                json!({"schemaVersion": 1, "kind": "sentence", "translation": "translation", "examples": [], "notes": []})
            )
        );
    }

    // 旧 dictionary.test.js：不合格的结构按原文收藏。
    #[test]
    fn malformed_objects_never_become_details() {
        for value in [
            json!(null),
            json!([]),
            json!({}),
            json!({"translation": "not an explicit sentence"}),
            json!({"kind": "other"}),
            json!({"explanations": "bad"}),
        ] {
            let s = entry_snapshot("source", &[ai(value), google(json!("fallback"))], None);
            assert_eq!((s.translation.as_str(), s.detail), ("fallback", None));
        }
        let raw = "```json\n{\"kind\":\"sentence\",\"translation\":null}\n```";
        let s = entry_snapshot("source", &[ai(json!(raw))], None);
        assert_eq!((s.translation.as_str(), s.detail), (raw, None));
    }

    #[test]
    fn priority_trusts_the_request_service() {
        let g = google(dictionary());
        let a = ai(sentence());
        assert_eq!(
            entry_snapshot("s", &[g.clone(), a.clone()], None).detail,
            Some(sentence())
        );
        assert_eq!(
            entry_snapshot("s", &[a.clone(), g.clone()], None).detail,
            Some(sentence())
        );
        let failed = item("ai@one", true, None);
        assert_eq!(
            entry_snapshot("s", &[g.clone(), failed], None).detail,
            Some(dictionary())
        );
        let mut bare = sentence();
        bare["examples"] = json!([]);
        bare["notes"] = json!([]);
        assert_eq!(
            entry_snapshot("s", &[g.clone(), ai(bare.clone())], None).detail,
            Some(dictionary())
        );
        // 只有解析或标记的 AI 句子也胜过排在前面的谷歌纯译文
        for (k, v) in [
            ("category", json!("EN01")),
            ("difficulty", json!(2)),
            ("nuance_note", json!("Tone.")),
            ("syntax_breakdown", json!({"main_clause": "Main"})),
            (
                "key_vocabulary",
                json!([{"word": "term", "meaning_in_context": "术语"}]),
            ),
        ] {
            let mut r = bare.clone();
            r[k] = v;
            let s = entry_snapshot("s", &[google(json!("plain")), ai(r)], None);
            assert_eq!(s.detail.unwrap()["kind"], "sentence", "{k}");
        }
        assert_eq!(
            entry_snapshot("s", &[google(json!("plain")), ai(bare)], None).detail,
            None
        );
        // 模型在结果里自称 ai 不算数
        let mut claims = sentence();
        claims["serviceName"] = "ai".into();
        assert_eq!(
            entry_snapshot(
                "s",
                &[g.clone(), item("plugin@one", false, Some(claims))],
                None
            )
            .detail,
            Some(dictionary())
        );
        // 旧词典形状的 AI 结果不是「新结构」，和谷歌词典同级，按顺序
        let mut legacy_ai = dictionary();
        legacy_ai["examples"] = sentence()["examples"].clone();
        assert_eq!(
            entry_snapshot("s", &[g.clone(), ai(legacy_ai)], None).detail,
            Some(dictionary())
        );
        let first = json!({"kind": "sentence", "translation": "First translation"});
        assert_eq!(
            entry_snapshot(
                "s",
                &[google(first.clone()), google(json!("later text"))],
                None
            )
            .translation,
            "First translation"
        );
        assert_eq!(
            entry_snapshot("s", &[google(json!("first text")), google(first)], None).translation,
            "first text"
        );
        assert_eq!(
            entry_snapshot("s", &[google(json!("first text")), g], None).detail,
            Some(dictionary())
        );
    }

    #[test]
    fn clicked_row_wins_when_it_has_a_result() {
        let items = [google(dictionary()), ai(sentence())];
        assert_eq!(
            entry_snapshot("s", &items, Some(0)).detail,
            Some(dictionary())
        );
        assert_eq!(
            entry_snapshot("s", &[item("g", false, None), ai(sentence())], Some(0)).detail,
            Some(sentence())
        );
    }

    #[test]
    fn completion_order_does_not_change_the_saved_entry() {
        for google_first in [true, false] {
            let mut e = SavedEntry::new(
                "source",
                vec![item("google", false, None), item("ai", true, None)],
            );
            let (first, second) = if google_first { (0, 1) } else { (1, 0) };
            let value = |row: usize| if row == 0 { dictionary() } else { sentence() };
            assert_eq!(e.patch(first, Some(value(first))), None, "收藏前不写库");
            e.save(None);
            let last = e.patch(second, Some(value(second))).unwrap();
            assert_eq!(last.detail, Some(sentence()));
        }
    }

    #[test]
    fn multiple_ai_instances_keep_the_configured_order() {
        let mut first = sentence();
        first["translation"] = "First configured AI".into();
        let mut second = sentence();
        second["translation"] = "Second configured AI".into();
        for reversed in [false, true] {
            let mut e = SavedEntry::new(
                "source",
                vec![
                    google(dictionary()),
                    item("ai@first", true, None),
                    item("ai@second", true, None),
                ],
            );
            e.save(None);
            let order = if reversed { [2, 1] } else { [1, 2] };
            let mut last = None;
            for row in order {
                let v = if row == 1 {
                    first.clone()
                } else {
                    second.clone()
                };
                last = e.patch(row, Some(v));
            }
            let last = last.unwrap();
            assert_eq!(last.detail, Some(first.clone()));
            assert!(!last.translation.contains("Second configured AI"));
            assert_eq!(last.service, "ai@first");
        }
    }

    #[test]
    fn late_result_after_save_updates_the_snapshot() {
        let mut e = SavedEntry::new("translation", vec![item("ai", true, None)]);
        let first = e.save(None);
        assert_eq!(first.translation, "");
        assert!(e.requested());
        let late = e.patch(0, Some(dictionary())).unwrap();
        assert_eq!(late.detail, Some(dictionary()));
        assert_eq!(e.kind(), Kind::Word);
    }
}
