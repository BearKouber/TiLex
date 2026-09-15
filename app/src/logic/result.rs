//! 翻译结果的形状（从旧 `translation_result.js`、`dictionary.js`、`saved_entry.js` 的展示部分、
//! `wordbook_format.js` 的 `isWord` / `extractJson` 移植）。
//!
//! 服务返回的结果是一个 `serde_json::Value`：字符串 = 纯文本；对象 = 通过 [`normalize_result`] 的词句结构。
//! 模型给的 JSON 只有校验通过才进渲染、复制、收藏和缓存；不合格的保留原文（旧规范 translation-reliability）。
//! 界面只消费 [`entry_display`] 的强类型投影。

use serde_json::{Map, Value, json};

/// 长难句分类。顺序就是导出分组顺序；码只能新增，不能改号或复用（生词本存的是码）。
pub struct Category {
    pub code: &'static str,
    pub lang: &'static str,
    pub no: &'static str,
    pub name: &'static str,
}

const fn cat(
    code: &'static str,
    lang: &'static str,
    no: &'static str,
    name: &'static str,
) -> Category {
    Category {
        code,
        lang,
        no,
        name,
    }
}

pub const CATEGORIES: [Category; 11] = [
    cat("EN01", "英文", "01", "定语从句类"),
    cat("EN02", "英文", "02", "状语从句类"),
    cat("EN03", "英文", "03", "名词性从句类"),
    cat("EN04", "英文", "04", "非谓语与独立主格"),
    cat("EN05", "英文", "05", "特殊句式（倒装/强调）"),
    cat("EN06", "英文", "06", "并列与长插入语"),
    cat("ZH01", "中文", "01", "多重长定语类"),
    cat("ZH02", "中文", "02", "长状语阻隔类"),
    cat("ZH03", "中文", "03", "多层复句嵌套类"),
    cat("ZH04", "中文", "04", "流水句与主语隐换类"),
    cat("ZH05", "中文", "05", "长连谓与兼语句"),
];

pub fn category(code: &str) -> Option<&'static Category> {
    CATEGORIES.iter().find(|c| c.code == code)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Word,
    Sentence,
}

impl Kind {
    /// 按原文分：[`is_word`] 为真是单词。AI 的期望结构、入库的 kind、生词本布局三处共用。
    pub fn of(text: &str) -> Kind {
        if is_word(text) {
            Kind::Word
        } else {
            Kind::Sentence
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Word => "word",
            Kind::Sentence => "sentence",
        }
    }
}

/// `\p{Script=Han}` 的常用范围（统一表意文字及扩展、兼容表意、部首、々〆〇）。
pub fn is_han(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x2E99 | 0x2E9B..=0x2EF3 | 0x2F00..=0x2FD5 | 0x3005 | 0x3007 | 0x3021..=0x3029
        | 0x3038..=0x303B | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF
        | 0x20000..=0x2FA1F | 0x30000..=0x323AF)
}

/// 词句分流：按空白分词 ≤ 2 且末尾不是句末标点 → 词。
/// 中文没有空格：带中文逗号/顿号/分号/冒号，或汉字超过 10 个，就算句子。
pub fn is_word(text: &str) -> bool {
    let t = text.trim();
    if t.contains(['，', '、', '；', '：']) || t.chars().filter(|c| is_han(*c)).count() > 10 {
        return false;
    }
    t.split_whitespace().count() <= 2 && !t.ends_with(['.', '?', '!', '。', '？', '！'])
}

/// 文本里第一个 `{` 到最后一个 `}` 当 JSON 解析（模型在 JSON 前后加了话的情况）。
pub fn extract_json(raw: &str) -> Option<Value> {
    let start = raw.find('{')?;
    let end = raw.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&raw[start..=end]).ok()
}

/// 模型输出 → 通过校验的词句结构，否则原文（`Value::String`）。
/// 先解析完整 JSON（可剥掉 ```json 围栏），失败才截取大括号：先截会把合法的顶层 `[词典]` 拆成对象，绕过顶层约定。
pub fn dictionary_result(text: &str, expected: Kind) -> Value {
    let whole = strip_fence(text.trim()).unwrap_or(text);
    serde_json::from_str::<Value>(whole)
        .ok()
        .or_else(|| extract_json(text))
        .and_then(|v| normalize_result(&v, Some(expected)))
        .unwrap_or_else(|| Value::String(text.to_owned()))
}

/// ```` ```json ... ``` ```` → 中间的内容（`json` 不分大小写、可省略）。
fn strip_fence(t: &str) -> Option<&str> {
    let inner = t.strip_prefix("```")?.strip_suffix("```")?;
    let inner = match inner.get(..4) {
        Some(tag) if tag.eq_ignore_ascii_case("json") => &inner[4..],
        _ => inner,
    };
    Some(inner.trim())
}

fn nonblank(v: Option<&Value>) -> Option<&str> {
    v.and_then(Value::as_str).filter(|s| !s.trim().is_empty())
}

fn strings(v: &Value) -> bool {
    v.as_array().is_some_and(|a| a.iter().all(Value::is_string))
}

fn objects(v: &Value, ok: impl Fn(&Map<String, Value>) -> bool) -> bool {
    v.as_array()
        .is_some_and(|a| a.iter().all(|i| i.as_object().is_some_and(&ok)))
}

fn absent_or(v: Option<&Value>, ok: impl Fn(&Value) -> bool) -> bool {
    v.is_none_or(ok)
}

fn nonblank_strings(v: Option<&Value>) -> Vec<Value> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|s| nonblank(Some(s)).is_some())
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// 服务结果、生词本记录、展示投影的共同边界。返回规范化后的对象，不合格返回 `None`。
/// 没有 `kind` 的旧词典故意不加 kind / schemaVersion：只有明确的新结构才能在收藏时拿到「AI 补充」优先级。
pub fn normalize_result(value: &Value, expected: Option<Kind>) -> Option<Value> {
    let v = value.as_object()?;
    let kind = match v.get("kind") {
        None => None,
        Some(Value::String(s)) if s == "word" => Some(Kind::Word),
        Some(Value::String(s)) if s == "sentence" => Some(Kind::Sentence),
        Some(_) => return None,
    };
    if expected.is_some_and(|e| kind.unwrap_or(Kind::Word) != e) {
        return None;
    }

    let valid = absent_or(v.get("pronunciations"), |p| {
        objects(p, |i| i.get("symbol").is_some_and(Value::is_string))
    }) && absent_or(v.get("explanations"), |e| {
        objects(e, |i| {
            i.get("explains").is_some_and(strings) && absent_or(i.get("trait"), Value::is_string)
        })
    }) && absent_or(v.get("associations"), strings)
        && absent_or(v.get("notes"), strings)
        && absent_or(v.get("examples"), |e| {
            objects(e, |i| {
                nonblank(i.get("text")).is_some() && nonblank(i.get("translation")).is_some()
            })
        })
        && absent_or(v.get("translation"), Value::is_string)
        && absent_or(v.get("syntax_breakdown"), |s| {
            s.is_null()
                || s.as_object().is_some_and(|o| {
                    absent_or(o.get("main_clause"), Value::is_string)
                        && absent_or(o.get("clauses_and_modifiers"), Value::is_string)
                })
        })
        && absent_or(v.get("nuance_note"), |n| n.is_null() || n.is_string())
        && absent_or(v.get("key_vocabulary"), |k| {
            k.is_null()
                || objects(k, |i| {
                    i.get("word").is_some_and(Value::is_string)
                        && i.get("meaning_in_context").is_some_and(Value::is_string)
                })
        });
    if !valid {
        return None;
    }

    let examples: Vec<Value> = v
        .get("examples")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|e| json!({ "text": e["text"], "translation": e["translation"] }))
                .collect()
        })
        .unwrap_or_default();
    let notes = nonblank_strings(v.get("notes"));

    if kind == Some(Kind::Sentence) {
        let translation = nonblank(v.get("translation"))?;
        let mut out = json!({
            "schemaVersion": 1,
            "kind": "sentence",
            "translation": translation,
            "examples": examples,
            "notes": notes,
        });
        // 标记是次要信息：非法的只丢它自己，不能连累译文。
        let category = v
            .get("category")
            .and_then(Value::as_str)
            .map(|c| c.trim().to_ascii_uppercase())
            .and_then(|c| self::category(&c));
        if let Some(c) = category {
            out["category"] = c.code.into();
        }
        let difficulty = match v.get("difficulty") {
            Some(Value::Number(n)) => n.as_f64(),
            Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
            _ => None,
        };
        if let Some(d) = difficulty.filter(|d| [1.0, 2.0, 3.0].contains(d)) {
            out["difficulty"] = (d as u8).into();
            if let Some(reason) = nonblank(v.get("difficulty_reason")) {
                out["difficulty_reason"] = reason.trim().into();
            }
        }
        if let Some(syntax) = v.get("syntax_breakdown").and_then(Value::as_object) {
            let main = nonblank(syntax.get("main_clause")).map(str::trim);
            let mods = nonblank(syntax.get("clauses_and_modifiers")).map(str::trim);
            if main.is_some() || mods.is_some() {
                out["syntax_breakdown"] = json!({
                    "main_clause": main.unwrap_or(""),
                    "clauses_and_modifiers": mods.unwrap_or(""),
                });
            }
        }
        if let Some(note) = nonblank(v.get("nuance_note")) {
            out["nuance_note"] = note.trim().into();
        }
        let vocab: Vec<Value> = v
            .get("key_vocabulary")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|i| {
                        let word = nonblank(i.get("word"))?.trim();
                        let meaning = nonblank(i.get("meaning_in_context"))?.trim();
                        Some(json!({ "word": word, "meaning_in_context": meaning }))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !vocab.is_empty() {
            out["key_vocabulary"] = vocab.into();
        }
        return Some(out);
    }

    let explanations: Vec<Value> = v
        .get("explanations")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    let explains = nonblank_strings(e.get("explains"));
                    if explains.is_empty() {
                        return None;
                    }
                    let mut item = json!({ "explains": explains });
                    if let Some(t) = e.get("trait") {
                        item["trait"] = t.clone();
                    }
                    Some(item)
                })
                .collect()
        })
        .unwrap_or_default();
    if explanations.is_empty() {
        return None;
    }
    let pronunciations: Vec<Value> = v
        .get("pronunciations")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| nonblank(p.get("symbol")))
                .map(|s| json!({ "symbol": s }))
                .collect()
        })
        .unwrap_or_default();
    let mut out = json!({
        "pronunciations": pronunciations,
        "explanations": explanations,
        "associations": nonblank_strings(v.get("associations")),
    });
    if kind == Some(Kind::Word) {
        out["schemaVersion"] = 1.into();
        out["kind"] = "word".into();
        out["examples"] = examples.into();
        out["notes"] = notes.into();
    } else {
        if v.contains_key("examples") {
            out["examples"] = examples.into();
        }
        if v.contains_key("notes") {
            out["notes"] = notes.into();
        }
    }
    Some(out)
}

fn str_list(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// 规范化结构的完整纯文本：复制、搜索、生词本 translation 列用。
fn structured_text(v: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    if v["kind"] == "sentence" {
        lines.extend(v["translation"].as_str().map(str::to_owned));
    } else {
        for p in v["pronunciations"].as_array().into_iter().flatten() {
            lines.extend(p["symbol"].as_str().map(str::to_owned));
        }
        for e in v["explanations"].as_array().into_iter().flatten() {
            let explains = str_list(e, "explains").join(", ");
            let line = format!("{} {explains}", e["trait"].as_str().unwrap_or(""));
            lines.push(line.trim().to_owned());
        }
        lines.extend(str_list(v, "associations"));
    }
    for e in v["examples"].as_array().into_iter().flatten() {
        lines.extend(
            [&e["text"], &e["translation"]]
                .iter()
                .filter_map(|x| x.as_str())
                .map(str::to_owned),
        );
    }
    lines.extend(str_list(v, "notes"));
    lines.retain(|l| !l.is_empty());
    lines.join("\n")
}

/// 结果的完整纯文本（复制 / 朗读 / 判断「有没有结果」）。字符串去首尾空白；结构走 [`structured_text`]；
/// 不合格的对象是空串。
pub fn result_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.trim().to_owned(),
        _ => normalize_result(value, None)
            .map(|v| structured_text(&v))
            .unwrap_or_default(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayKind {
    Word,
    Sentence,
    /// 纯文本（谷歌整句、不合格的 AI 原文、坏掉的 detail）。
    Text,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Explanation {
    /// 词性，没有就是空串。
    pub part: String,
    pub explains: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Example {
    pub text: String,
    pub translation: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Syntax {
    pub main_clause: String,
    pub clauses_and_modifiers: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Vocab {
    pub word: String,
    pub meaning_in_context: String,
}

/// 浮窗、生词本、导出共用的唯一展示投影。每个栏目只出现一次；空栏目界面不出标题。
#[derive(Clone, Debug, PartialEq)]
pub struct EntryDisplay {
    pub kind: DisplayKind,
    /// 主译文：单词是空（释义已经分栏显示），句子是结构里的译文，纯文本就是原样。
    pub translation: String,
    pub pronunciations: Vec<String>,
    pub explanations: Vec<Explanation>,
    pub associations: Vec<String>,
    pub examples: Vec<Example>,
    pub notes: Vec<String>,
    /// 分类码（`EN01` …），查 [`category`] 取名字。
    pub category: Option<String>,
    pub difficulty: Option<u8>,
    pub difficulty_reason: String,
    pub syntax_breakdown: Option<Syntax>,
    pub nuance_note: String,
    pub key_vocabulary: Vec<Vocab>,
}

/// `detail` 是结构（服务结果或生词本 detail 列），`translation` 是纯文本（结果文本或生词本 translation 列）。
/// 浮窗每行：`entry_display(Some(&v), &result_text(&v))`。
pub fn entry_display(detail: Option<&Value>, translation: &str) -> EntryDisplay {
    let raw = detail.filter(|d| d.is_object());
    let d = raw.and_then(|r| normalize_result(r, None));
    let kind = match &d {
        None => DisplayKind::Text,
        Some(d) if d["kind"] == "sentence" => DisplayKind::Sentence,
        Some(_) => DisplayKind::Word,
    };
    let empty = Value::Null;
    let n = d.as_ref().unwrap_or(&empty);
    let mut associations = str_list(n, "associations");
    // 旧收藏没存搭配，但平铺译文里词典之后就是搭配：前缀对得上才恢复。明确的空数组保持空。
    if kind == DisplayKind::Word
        && let Some(r) = raw
        && r.get("kind").is_none()
        && r.get("associations").is_none_or(Value::is_null)
    {
        let mut without = n.clone();
        without["associations"] = json!([]);
        let prefix = format!("{}\n", structured_text(&without));
        if let Some(rest) = translation.strip_prefix(&prefix) {
            associations = rest
                .split('\n')
                .filter(|l| !l.trim().is_empty())
                .map(str::to_owned)
                .collect();
        }
    }
    let raw_obj = raw.unwrap_or(&empty);
    let syntax = raw_obj.get("syntax_breakdown").filter(|s| s.is_object());
    let main = syntax.and_then(|s| s["main_clause"].as_str()).unwrap_or("");
    let mods = syntax
        .and_then(|s| s["clauses_and_modifiers"].as_str())
        .unwrap_or("");
    EntryDisplay {
        kind,
        translation: match kind {
            DisplayKind::Word => String::new(),
            DisplayKind::Sentence => n["translation"].as_str().unwrap_or("").to_owned(),
            DisplayKind::Text => translation.to_owned(),
        },
        pronunciations: n["pronunciations"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| p["symbol"].as_str().map(str::to_owned))
            .collect(),
        explanations: n["explanations"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| Explanation {
                part: e["trait"].as_str().unwrap_or("").to_owned(),
                explains: str_list(e, "explains"),
            })
            .collect(),
        associations,
        examples: n["examples"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| Example {
                text: e["text"].as_str().unwrap_or("").to_owned(),
                translation: e["translation"].as_str().unwrap_or("").to_owned(),
            })
            .collect(),
        notes: str_list(n, "notes"),
        category: n["category"].as_str().map(str::to_owned),
        difficulty: n["difficulty"].as_u64().and_then(|d| u8::try_from(d).ok()),
        difficulty_reason: n["difficulty_reason"].as_str().unwrap_or("").to_owned(),
        // 历史分析直接读原始 detail（旧记录没有 kind 也要显示），只取类型对的部分。
        syntax_breakdown: (!main.trim().is_empty() || !mods.trim().is_empty()).then(|| Syntax {
            main_clause: main.to_owned(),
            clauses_and_modifiers: mods.to_owned(),
        }),
        nuance_note: raw_obj["nuance_note"].as_str().unwrap_or("").to_owned(),
        key_vocabulary: raw_obj["key_vocabulary"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|i| {
                let word = nonblank(i.get("word"))?;
                let meaning = nonblank(i.get("meaning_in_context"))?;
                Some(Vocab {
                    word: word.to_owned(),
                    meaning_in_context: meaning.to_owned(),
                })
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_dictionary() -> Value {
        json!({
            "explanations": [{"trait": "n.", "explains": ["word"]}],
            "pronunciations": [{"symbol": "/w/"}],
            "associations": ["words"],
        })
    }

    // ---- 旧 dictionary.test.js ----

    #[test]
    fn dictionary_result_accepts_whole_and_fenced_json_only() {
        let valid = valid_dictionary();
        let text = valid.to_string();
        assert_eq!(dictionary_result(&text, Kind::Word), valid);
        assert_eq!(
            dictionary_result(&format!("```json\n{text}\n```"), Kind::Word),
            valid
        );
        assert_eq!(
            dictionary_result(&format!("```JSON {text} ```"), Kind::Word),
            valid
        );
        // 顶层数组不解包
        for wrapped in [format!("[{text}]"), format!("```json\n[{text}]\n```")] {
            assert_eq!(dictionary_result(&wrapped, Kind::Word), json!(wrapped));
        }
        let empty = json!({"explanations": [{"explains": []}]}).to_string();
        assert_eq!(dictionary_result(&empty, Kind::Word), json!(empty));
        assert_eq!(dictionary_result("not JSON", Kind::Word), json!("not JSON"));
        assert!(result_text(&dictionary_result(&text, Kind::Word)).contains("word"));
    }

    #[test]
    fn malformed_dictionaries_stay_verbatim_text() {
        let valid = valid_dictionary();
        let mut bad = vec![
            json!(null),
            json!([]),
            json!(true),
            json!("string"),
            json!({}),
            json!({"explanations": []}),
            json!({"explanations": {}}),
            json!({"explanations": [null]}),
            json!({"explanations": ["word"]}),
            json!({"explanations": [{"explains": "word"}]}),
            json!({"explanations": [{"explains": [{}]}]}),
            json!({"explanations": [{"explains": ["word"], "trait": {}}]}),
        ];
        for p in [
            json!(null),
            json!({}),
            json!([null]),
            json!([{}]),
            json!([{"symbol": {}}]),
        ] {
            let mut v = valid.clone();
            v["pronunciations"] = p;
            bad.push(v);
        }
        for a in [
            json!(null),
            json!({}),
            json!("word"),
            json!([null]),
            json!([{}]),
        ] {
            let mut v = valid.clone();
            v["associations"] = a;
            bad.push(v);
        }
        for value in bad {
            let text = value.to_string();
            let result = dictionary_result(&text, Kind::Word);
            assert_eq!(result, json!(text), "{text}");
            assert_eq!(result_text(&result), text);
        }
    }

    #[test]
    fn sentence_results_need_the_sentence_kind() {
        let sentence = json!({
            "kind": "sentence",
            "translation": "完整译文",
            "examples": [{"text": "A sentence.", "translation": "一个句子。"}],
            "notes": ["用法说明"],
        });
        let mut expected = sentence.clone();
        expected["schemaVersion"] = 1.into();
        let text = sentence.to_string();
        assert_eq!(dictionary_result(&text, Kind::Sentence), expected);
        assert_eq!(dictionary_result(&text, Kind::Word), json!(text));
        let dict = valid_dictionary().to_string();
        assert_eq!(dictionary_result(&dict, Kind::Sentence), json!(dict));
        assert_eq!(
            dictionary_result(&format!("Here is the translation: {text}"), Kind::Sentence),
            expected
        );
        for raw in [
            format!("[{text}]"),
            format!("```json\n[{text}]\n```"),
            " \nnot JSON\n ".to_owned(),
            "\"ordinary text\"".to_owned(),
        ] {
            assert_eq!(dictionary_result(&raw, Kind::Sentence), json!(raw));
        }
    }

    // ---- 旧 translation_result.test.js ----

    fn word() -> Value {
        json!({
            "kind": "word",
            "schemaVersion": 999,
            "pronunciations": [{"symbol": "/wɜːd/", "voice": "ignored"}],
            "explanations": [{"trait": "n.", "explains": ["word", ""]}],
            "associations": ["word choice", " "],
            "examples": [{"text": "Choose a word.", "translation": "选一个词。"}],
            "notes": ["Usage", " "],
        })
    }

    #[test]
    fn normalizes_words_and_sentences() {
        let n = normalize_result(&word(), Some(Kind::Word)).unwrap();
        assert_eq!(n["schemaVersion"], 1);
        assert_eq!(
            n["explanations"],
            json!([{"trait": "n.", "explains": ["word"]}])
        );
        assert_eq!(n["pronunciations"], json!([{"symbol": "/wɜːd/"}]));
        assert_eq!(n["associations"], json!(["word choice"]));
        assert_eq!(n["notes"], json!(["Usage"]));
        assert_eq!(normalize_result(&n, None).unwrap(), n, "规范化是幂等的");
        assert_eq!(
            normalize_result(
                &json!({"kind": "sentence", "translation": "Complete."}),
                None
            )
            .unwrap(),
            json!({"kind": "sentence", "schemaVersion": 1, "translation": "Complete.", "examples": [], "notes": []})
        );
        assert_eq!(
            normalize_result(
                &json!({"kind": "word", "explanations": [{"explains": ["meaning"]}]}),
                None
            )
            .unwrap(),
            json!({"kind": "word", "schemaVersion": 1, "explanations": [{"explains": ["meaning"]}],
                   "pronunciations": [], "associations": [], "examples": [], "notes": []})
        );
    }

    #[test]
    fn legacy_dictionary_keeps_its_legacy_identity() {
        let legacy = normalize_result(
            &json!({"explanations": [{"explains": ["meaning"]}], "sentence": []}),
            None,
        )
        .unwrap();
        assert!(legacy.get("kind").is_none() && legacy.get("schemaVersion").is_none());
        assert_eq!(normalize_result(&legacy, Some(Kind::Sentence)), None);
        assert_eq!(normalize_result(&word(), Some(Kind::Sentence)), None);
        assert_eq!(
            normalize_result(
                &json!({"kind": "sentence", "translation": "Complete."}),
                Some(Kind::Word)
            ),
            None
        );
        let mut with_supplements = legacy.clone();
        with_supplements["examples"] = word()["examples"].clone();
        with_supplements["notes"] = json!(["Usage"]);
        assert_eq!(
            normalize_result(&with_supplements, None).unwrap()["examples"],
            word()["examples"]
        );
    }

    #[test]
    fn invalid_shapes_are_rejected() {
        let mut invalid = vec![
            json!(null),
            json!([]),
            json!(true),
            json!("text"),
            json!({}),
            json!({"kind": "other"}),
            json!({"kind": null}),
            json!({"kind": "sentence"}),
            json!({"kind": "sentence", "translation": ""}),
            json!({"kind": "sentence", "translation": " \n"}),
            json!({"kind": "sentence", "translation": 3}),
            json!({"explanations": []}),
            json!({"explanations": [{"explains": [" ", ""]}]}),
            json!({"kind": "sentence", "translation": "Complete.", "pronunciations": "bad"}),
        ];
        let with = |key: &str, value: Value| {
            let mut w = word();
            w[key] = value;
            w
        };
        for e in [
            json!(null),
            json!({}),
            json!(["text"]),
            json!([null]),
            json!([{"explains": false}]),
            json!([{"explains": [3]}]),
            json!([{"trait": [], "explains": ["valid"]}]),
        ] {
            invalid.push(with("explanations", e));
        }
        for p in [
            json!(null),
            json!({}),
            json!("bad"),
            json!([null]),
            json!([{"symbol": []}]),
        ] {
            invalid.push(with("pronunciations", p));
        }
        for key in ["associations", "notes"] {
            for a in [
                json!(null),
                json!({}),
                json!("bad"),
                json!([null]),
                json!([3]),
            ] {
                invalid.push(with(key, a));
            }
        }
        for e in [
            json!(null),
            json!({}),
            json!("bad"),
            json!([null]),
            json!([{}]),
            json!([{"text": "x"}]),
            json!([{"text": "", "translation": "x"}]),
            json!([{"text": "x", "translation": "  "}]),
            json!([{"text": "x", "translation": {}}]),
        ] {
            invalid.push(with("examples", e));
        }
        for value in invalid {
            assert_eq!(normalize_result(&value, None), None, "{value}");
        }
    }

    #[test]
    fn sentence_tags_drop_alone() {
        let base = json!({"kind": "sentence", "translation": "Complete."});
        let tagged = |extra: Value| {
            let mut v = base.clone();
            for (k, x) in extra.as_object().unwrap() {
                v[k] = x.clone();
            }
            normalize_result(&v, None).unwrap()
        };
        let t = tagged(
            json!({"category": " zh03 ", "difficulty": "2", "difficulty_reason": " 主谓分离 "}),
        );
        assert_eq!(t["category"], "ZH03");
        assert_eq!(t["difficulty"], 2);
        assert_eq!(t["difficulty_reason"], "主谓分离");
        assert_eq!(normalize_result(&t, None).unwrap(), t);
        assert_eq!(CATEGORIES.len(), 11);
        assert_eq!([CATEGORIES[5].code, CATEGORIES[6].code], ["EN06", "ZH01"]);
        for category in [
            json!("EN07"),
            json!("ZH06"),
            json!(""),
            json!(3),
            json!(null),
            json!({}),
            json!("toString"),
            json!("定语从句类"),
        ] {
            let r = tagged(json!({"category": category, "difficulty": 1}));
            assert_eq!(r["translation"], "Complete.");
            assert!(r.get("category").is_none(), "{category}");
            assert_eq!(r["difficulty"], 1);
        }
        for difficulty in [
            json!(0),
            json!(4),
            json!(1.5),
            json!(-1),
            json!(""),
            json!("hard"),
            json!(true),
            json!(null),
            json!([2]),
            json!({}),
        ] {
            let r = tagged(
                json!({"category": "EN01", "difficulty": difficulty, "difficulty_reason": "why"}),
            );
            assert_eq!(r["category"], "EN01");
            assert!(r.get("difficulty").is_none(), "{difficulty}");
            assert!(
                r.get("difficulty_reason").is_none(),
                "reason needs a valid difficulty"
            );
        }
        assert!(
            tagged(json!({"difficulty": 3, "difficulty_reason": " "}))
                .get("difficulty_reason")
                .is_none()
        );
        assert!(
            tagged(json!({"difficulty": 3, "difficulty_reason": 7}))
                .get("difficulty_reason")
                .is_none()
        );
        assert_eq!(tagged(json!({"difficulty": 2.0}))["difficulty"], 2);
        let mut w = word();
        w["category"] = "EN01".into();
        assert!(
            normalize_result(&w, None)
                .unwrap()
                .get("category")
                .is_none(),
            "words carry no sentence tags"
        );
    }

    #[test]
    fn sentence_analysis_is_trimmed_and_optional() {
        let v = json!({
            "kind": "sentence", "translation": "T",
            "syntax_breakdown": {"main_clause": " Main ", "clauses_and_modifiers": " "},
            "nuance_note": " Tone. ",
            "key_vocabulary": [
                {"word": " term ", "meaning_in_context": " 术语 "},
                {"word": "x", "meaning_in_context": ""}
            ],
        });
        let n = normalize_result(&v, None).unwrap();
        assert_eq!(
            n["syntax_breakdown"],
            json!({"main_clause": "Main", "clauses_and_modifiers": ""})
        );
        assert_eq!(n["nuance_note"], "Tone.");
        assert_eq!(
            n["key_vocabulary"],
            json!([{"word": "term", "meaning_in_context": "术语"}])
        );
        let bare = normalize_result(
            &json!({"kind": "sentence", "translation": "T", "syntax_breakdown": null,
                    "nuance_note": null, "key_vocabulary": null}),
            None,
        )
        .unwrap();
        for key in ["syntax_breakdown", "nuance_note", "key_vocabulary"] {
            assert!(bare.get(key).is_none(), "{key}");
        }
        assert_eq!(
            normalize_result(
                &json!({"kind": "sentence", "translation": "T", "syntax_breakdown": {"main_clause": 1}}),
                None
            ),
            None
        );
    }

    // ---- isWord / extractJson（旧 wordbook_format.test.js 的那两段）----

    #[test]
    fn word_or_sentence() {
        for w in [
            "hello",
            "give up",
            "  book  ",
            "实现",
            "画蛇添足",
            "中华人民共和国",
            "十个汉字刚好不算句子",
        ] {
            assert!(is_word(w), "{w}");
        }
        for s in [
            "Hello.",
            "I love you",
            "这是一个句子。",
            "项目上线后，性能明显提升",
            "安装、配置",
            "注意：先备份",
            "在数字化转型背景下企业需要持续投入",
        ] {
            assert!(!is_word(s), "{s}");
        }
        assert_eq!(Kind::of("项目组在实现过程中发现了新的问题"), Kind::Sentence);
        assert_eq!(Kind::of("实现"), Kind::Word);
    }

    #[test]
    fn extract_json_takes_the_outer_braces() {
        assert_eq!(
            extract_json("```json\n{\"a\":1}\n```"),
            Some(json!({"a": 1}))
        );
        assert_eq!(
            extract_json("好的，结果如下：{\"a\":[1,2]} 完毕"),
            Some(json!({"a": [1, 2]}))
        );
        assert_eq!(extract_json("抱歉我不能"), None);
        assert_eq!(extract_json("{坏掉的}"), None);
        assert_eq!(extract_json("} {"), None);
    }

    // ---- 旧 saved_entry.test.js 里 entryDisplay / resultText 的部分 ----

    fn dictionary() -> Value {
        json!({
            "pronunciations": [{"symbol": "/a/"}, {"symbol": "/b/"}],
            "explanations": [{"trait": "n.", "explains": ["translation"]}],
            "associations": ["machine translation", "literal translation"],
        })
    }

    fn full(kind: &str) -> Value {
        let mut v = if kind == "word" {
            dictionary()
        } else {
            json!({"translation": "完整译文。"})
        };
        v["schemaVersion"] = 1.into();
        v["kind"] = kind.into();
        v["examples"] = json!([{"text": "Literal example.", "translation": "例句译文。"}]);
        v["notes"] = json!(["Usage note.", "Grammar note."]);
        v
    }

    #[test]
    fn dictionary_displays_meanings_once() {
        let d = dictionary();
        let display = entry_display(Some(&d), &result_text(&d));
        assert_eq!(display.kind, DisplayKind::Word);
        assert_eq!(display.translation, "");
        assert_eq!(
            display.associations,
            ["machine translation", "literal translation"]
        );
        let legacy =
            json!({"pronunciations": d["pronunciations"], "explanations": d["explanations"]});
        assert_eq!(entry_display(Some(&legacy), &result_text(&d)), display);
        assert_eq!(
            entry_display(None, "plain sentence").translation,
            "plain sentence"
        );
        assert!(
            entry_display(Some(&legacy), "unrelated translation\nnot a collocation")
                .associations
                .is_empty()
        );
    }

    #[test]
    fn full_results_render_each_section_once() {
        for kind in ["word", "sentence"] {
            let v = full(kind);
            let text = result_text(&v);
            let display = entry_display(Some(&v), &text);
            assert_eq!(display.examples.len(), 1);
            assert_eq!(display.notes, ["Usage note.", "Grammar note."]);
            let main = if kind == "word" {
                ""
            } else {
                "完整译文。"
            };
            assert_eq!(display.translation, main);
            for piece in [
                "Literal example.",
                "例句译文。",
                "Usage note.",
                "Grammar note.",
            ] {
                assert_eq!(text.matches(piece).count(), 1, "{piece}");
                assert!(!display.translation.contains(piece));
            }
        }
        assert_eq!(
            result_text(&full("sentence")),
            "完整译文。\nLiteral example.\n例句译文。\nUsage note.\nGrammar note."
        );
        assert_eq!(result_text(&json!("  plain  ")), "plain");
    }

    #[test]
    fn malformed_details_display_as_text() {
        let mut bad_notes = full("sentence");
        bad_notes["notes"] = json!([false]);
        let mut bad_translation = full("sentence");
        bad_translation["translation"] = json!({});
        for value in [
            json!(null),
            json!([]),
            json!({}),
            json!({"translation": "not an explicit sentence"}),
            bad_notes,
            bad_translation,
            json!({"kind": "other"}),
        ] {
            assert_eq!(result_text(&value), "");
            let d = entry_display(Some(&value), "raw readable response");
            assert_eq!(d.kind, DisplayKind::Text);
            assert_eq!(d.translation, "raw readable response");
            assert!(d.explanations.is_empty());
        }
    }

    #[test]
    fn legacy_analysis_is_safe() {
        let analysis = json!({
            "syntax_breakdown": {"main_clause": "Main clause", "clauses_and_modifiers": "Modifiers"},
            "nuance_note": "Legacy nuance",
            "key_vocabulary": [{"word": "word", "meaning_in_context": "Meaning"}],
        });
        let d = entry_display(Some(&analysis), "Legacy sentence");
        assert_eq!(d.kind, DisplayKind::Text);
        assert_eq!(d.translation, "Legacy sentence");
        assert_eq!(d.syntax_breakdown.unwrap().main_clause, "Main clause");
        assert_eq!(d.nuance_note, "Legacy nuance");
        assert_eq!(d.key_vocabulary[0].word, "word");
        let malformed = json!({
            "syntax_breakdown": {"main_clause": {}, "clauses_and_modifiers": null},
            "nuance_note": [],
            "key_vocabulary": [null, {"word": {}}, {"word": "safe", "meaning_in_context": {}}],
        });
        let m = entry_display(Some(&malformed), "readable");
        assert_eq!(m.syntax_breakdown, None);
        assert_eq!(m.nuance_note, "");
        assert!(m.key_vocabulary.is_empty());
    }

    #[test]
    fn sentence_tags_reach_the_display() {
        let mut v = full("sentence");
        v["category"] = "en02".into();
        v["difficulty"] = 2.into();
        v["difficulty_reason"] = "主谓被逗号隔开".into();
        let d = entry_display(Some(&v), "x");
        assert_eq!(d.category.as_deref(), Some("EN02"));
        assert_eq!(category("EN02").unwrap().name, "状语从句类");
        assert_eq!(d.difficulty, Some(2));
        assert_eq!(d.difficulty_reason, "主谓被逗号隔开");
        let mut broken = full("sentence");
        broken["category"] = "EN09".into();
        broken["difficulty"] = 9.into();
        assert_eq!(entry_display(Some(&broken), "x").translation, "完整译文。");
        for detail in [
            full("sentence"),
            full("word"),
            json!(null),
            json!("{bad"),
            json!({"syntax_breakdown": {"main_clause": "Old"}}),
        ] {
            let d = entry_display(Some(&detail), "plain");
            assert_eq!(
                (d.category, d.difficulty, d.difficulty_reason.as_str()),
                (None, None, "")
            );
        }
    }
}
