//! 生词本导出 Markdown（design §2.3，从旧 `wordbook_format.js` 移植）。
//! 纯函数：不读时钟、不写盘、不改入参。

use chrono::{DateTime, Local};

use crate::logic::result::{CATEGORIES, EntryDisplay, Example, Kind, entry_display, is_han};
use crate::logic::wordbook::Entry;

/// 我们自己插的换行（例句原句和译句之间），必须是硬换行，不然渲染出来会连成一行。
const HARD_BREAK: &str = "  \n";
/// **原文自带的换行只用普通换行接上**，不用 `"  \n"`：硬换行在 Typora 这类编辑器里
/// 每行尾都会画一个 ↵ 箭头，一整段全是箭头（用户看实际导出文件时提的）。
/// 代价是渲染时这几行会接成一段流式文字 —— 想渲染后也分行只能用 `<br>`，那是 HTML，另说。
const SOFT_BREAK: &str = "\n";
/// 表格单元格里换不了行：`\n` 会当场把表格截断。只有这一处用 `<br>`
/// （HTML，Typora / GitHub 都认），别的地方仍然走 [`SOFT_BREAK`]。
const CELL_BREAK: &str = "<br>";

/// 生词本导出成 Markdown。`now` 注入，不读时钟、不写盘、不改入参。
pub fn build_markdown(entries: &[Entry], now: DateTime<Local>) -> String {
    let mut words: Vec<WordItem> = entries
        .iter()
        .filter(|e| e.kind == Kind::Word)
        .map(|e| {
            let first_char = e.text.trim().chars().next().unwrap_or('\0');
            WordItem {
                entry: e,
                letter: letter_of(&e.text),
                han: is_han(first_char),
            }
        })
        .collect();

    words.sort_by(|a, b| {
        if a.letter != b.letter {
            if a.letter == '#' {
                return std::cmp::Ordering::Greater;
            }
            if b.letter == '#' {
                return std::cmp::Ordering::Less;
            }
            return a.letter.cmp(&b.letter);
        }
        if a.han != b.han {
            return if a.han {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            };
        }
        if a.han {
            pinyin_sort_key(&a.entry.text)
                .cmp(&pinyin_sort_key(&b.entry.text))
                .then_with(|| a.entry.text.cmp(&b.entry.text))
        } else {
            a.entry.text.cmp(&b.entry.text)
        }
    });

    let mut sentences: Vec<SentenceItem> = entries
        .iter()
        .filter(|e| e.kind != Kind::Word)
        .map(|e| {
            let view = entry_display(e.detail.as_ref(), &e.translation);
            let cat_idx = view
                .category
                .as_deref()
                .and_then(|code| CATEGORIES.iter().position(|c| c.code == code))
                .unwrap_or(CATEGORIES.len());
            SentenceItem {
                entry: e,
                view,
                category_index: cat_idx,
            }
        })
        .collect();

    sentences.sort_by(|a, b| {
        a.category_index
            .cmp(&b.category_index)
            .then_with(|| {
                let diff_a = a.view.difficulty.unwrap_or(4);
                let diff_b = b.view.difficulty.unwrap_or(4);
                diff_a.cmp(&diff_b)
            })
            .then_with(|| b.entry.created_at.cmp(&a.entry.created_at))
            .then_with(|| b.entry.id.cmp(&a.entry.id))
    });

    let mut out: Vec<String> = vec![
        "# 我的生词本".to_owned(),
        String::new(),
        format!(
            "> 导出于 {} · 单词 {} · 长难句 {}",
            now.format("%Y-%m-%d %H:%M"),
            words.len(),
            sentences.len()
        ),
        String::new(),
    ];

    if !words.is_empty() {
        out.push("## 单词".to_owned());
        out.push(String::new());
        let mut current_letter = '\0';
        for w in &words {
            if w.letter != current_letter {
                current_letter = w.letter;
                // `#` 原样写会变成标题，实体化掉
                out.push(if current_letter == '#' {
                    "### &#35;".to_owned()
                } else {
                    format!("### {current_letter}")
                });
                out.push(String::new());
            }
            out.push(format!(
                "#### 📖 **{}**",
                markdown_text(&one_line(&w.entry.text))
            ));
            out.push(String::new());
            let view = entry_display(w.entry.detail.as_ref(), &w.entry.translation);
            for block in word_blocks(&view, w.entry, None) {
                out.push(block);
                out.push(String::new());
            }
        }
    }

    if !sentences.is_empty() {
        out.push("## 长难句".to_owned());
        out.push(String::new());
        let mut current_cat_index: Option<usize> = None;
        for (index, s) in sentences.iter().enumerate() {
            if current_cat_index != Some(s.category_index) {
                current_cat_index = Some(s.category_index);
                let title = if s.category_index < CATEGORIES.len() {
                    let cat = &CATEGORIES[s.category_index];
                    format!("### {} · {} {}", cat.lang, cat.no, cat.name)
                } else {
                    "### 未分类".to_owned()
                };
                out.push(title);
                out.push(String::new());
            }

            let clean = squash_whitespace(&s.entry.text);
            let chars: Vec<char> = clean.chars().collect();
            let summary = if chars.len() > 35 {
                let cut = &chars[..35];
                let last_space = cut.iter().rposition(|&c| c == ' ');
                let mut sum: String = match last_space {
                    Some(pos) if pos > 20 => cut[..pos].iter().collect(),
                    _ => cut.iter().collect(),
                };
                sum.push_str("...");
                sum
            } else {
                clean
            };

            out.push(format!("#### {}. {}", index + 1, italic(&summary)));
            out.push(String::new());

            for block in sentence_blocks(&s.view, s.entry, &[]) {
                out.push(block);
                out.push(String::new());
            }
            out.push("---".to_owned());
            out.push(String::new());
        }
    }

    out.join("\n")
}

struct WordItem<'a> {
    entry: &'a Entry,
    letter: char,
    han: bool,
}

struct SentenceItem<'a> {
    entry: &'a Entry,
    view: EntryDisplay,
    category_index: usize,
}

fn has_kana(text: &str) -> bool {
    text.chars().any(|c| matches!(c, '\u{3040}'..='\u{30FF}'))
}

fn letter_of(text: &str) -> char {
    use pinyin::ToPinyin;
    let Some(c) = text.trim().chars().next() else {
        return '#';
    };
    if c.is_ascii_alphabetic() {
        return c.to_ascii_uppercase();
    }
    if !is_han(c) || has_kana(text) {
        return '#';
    }
    if let Some(first) = c
        .to_pinyin()
        .and_then(|py| py.first_letter().chars().next())
        .filter(|first| first.is_ascii_alphabetic())
    {
        return first.to_ascii_uppercase();
    }
    '#'
}

fn pinyin_sort_key(text: &str) -> String {
    use pinyin::ToPinyin;
    let mut out = String::new();
    for c in text.chars() {
        if let Some(py) = c.to_pinyin() {
            out.push_str(py.plain());
            out.push('\'');
        } else {
            out.push(c);
        }
    }
    out
}

/// 需要实体化的字符。旧版 `wordbook_format.js` 把**所有** ASCII 标点都转成 `&#NN;`，
/// 结果正常句子里的句号逗号分号全成了 `&#46;` `&#44;` `&#59;`，读起来满屏乱码（用户手测提的）。
/// 这里只留 Markdown 真会吃掉的那几个，其余标点原样输出。
fn needs_escape(c: char) -> bool {
    matches!(
        c,
        '\\' | '`' | '*' | '_' | '[' | ']' | '<' | '>' | '&' | '|' | '~'
    )
}

/// 行首才危险的块级标记：`#` 是标题，`-` / `+` 是无序列表，`>` 是引用（`>` 已在上面一律转义）。
/// 出现在行中间的这些字符没有语法含义，不动。
fn needs_escape_at_line_start(c: char) -> bool {
    matches!(c, '#' | '-' | '+' | '=')
}

fn markdown_text(value: &str) -> String {
    let normalized = value.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut processed_lines = Vec::with_capacity(lines.len());

    for line in lines {
        if line.is_empty() {
            processed_lines.push("&#8203;".to_owned());
            continue;
        }

        let mut column = 0usize;
        let mut expanded = String::new();
        for c in line.chars() {
            if c == '\t' {
                let width = 4 - (column % 4);
                column += width;
                for _ in 0..width {
                    expanded.push(' ');
                }
            } else {
                column += 1;
                expanded.push(c);
            }
        }

        let chars: Vec<char> = expanded.chars().collect();
        let total_len = chars.len();
        let mut line_out = String::new();
        let mut i = 0;

        while i < total_len {
            let c = chars[i];
            if needs_escape(c) || (i == 0 && needs_escape_at_line_start(c)) {
                line_out.push_str(&format!("&#{};", c as u32));
                i += 1;
            } else if c == ' ' {
                let start = i;
                while i < total_len && chars[i] == ' ' {
                    i += 1;
                }
                let run_len = i - start;
                if run_len > 1 || start == 0 || start + run_len == total_len {
                    for _ in 0..run_len {
                        line_out.push_str("&#160;");
                    }
                } else {
                    line_out.push(' ');
                }
            } else {
                line_out.push(c);
                i += 1;
            }
        }

        processed_lines.push(line_out);
    }

    processed_lines.join(SOFT_BREAK)
}

fn italic(value: &str) -> String {
    format!("*{}*", markdown_text(value.trim()))
}

fn replace_newline_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            let mut has_newline = false;
            let mut ws_run = String::new();
            while let Some(&next_c) = chars.peek() {
                if next_c.is_whitespace() {
                    if next_c == '\r' || next_c == '\n' {
                        has_newline = true;
                    }
                    ws_run.push(next_c);
                    chars.next();
                } else {
                    break;
                }
            }
            if has_newline {
                out.push(' ');
            } else {
                out.push_str(&ws_run);
            }
        } else {
            out.push(c);
            chars.next();
        }
    }
    out
}

fn code_text(value: &str) -> String {
    let without_backticks: String = value.chars().filter(|&c| c != '`').collect();
    replace_newline_whitespace(&without_backticks)
        .trim()
        .to_owned()
}

fn quote(text: &str) -> String {
    text.split('\n')
        .map(|line| format!("> {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn one_line(text: &str) -> String {
    replace_newline_whitespace(text.trim())
}

fn squash_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for c in s.trim().chars() {
        if c.is_whitespace() {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    out
}

fn example_blocks(examples: &[Example]) -> Vec<String> {
    if examples.is_empty() {
        return Vec::new();
    }
    let items: Vec<String> = examples
        .iter()
        .enumerate()
        .map(|(index, ex)| {
            let content = format!(
                "{}{}{}",
                markdown_text(&ex.text),
                HARD_BREAK,
                italic(&ex.translation)
            );
            let prefix = format!("{}. ", index + 1);
            let indent = " ".repeat(prefix.len());
            let indented = content.replace('\n', &format!("\n{indent}"));
            format!("{prefix}{indented}")
        })
        .collect();
    vec!["**例句**".to_owned(), items.join("\n")]
}

fn note_blocks(notes: &[String]) -> Vec<String> {
    if notes.is_empty() {
        return Vec::new();
    }
    let mut blocks = Vec::with_capacity(notes.len() + 1);
    blocks.push("**补充说明**".to_owned());
    for note in notes {
        blocks.push(markdown_text(note));
    }
    blocks
}

fn word_blocks(view: &EntryDisplay, w: &Entry, from_text: Option<&str>) -> Vec<String> {
    let mut blocks = Vec::new();

    let symbols: Vec<String> = view
        .pronunciations
        .iter()
        .map(|s| code_text(s))
        .filter(|s| !s.is_empty())
        .collect();
    if !symbols.is_empty() {
        let joined = symbols
            .into_iter()
            .map(|s| format!("`{s}`"))
            .collect::<Vec<_>>()
            .join(" · ");
        blocks.push(joined);
    }

    if !view.explanations.is_empty() {
        let mut rows = vec![
            "| 词性 | 详细释义 |".to_owned(),
            "| :---: | :--- |".to_owned(),
        ];
        for exp in &view.explanations {
            let clean_trait = if !exp.part.trim().is_empty() {
                let cleaned: String = exp
                    .part
                    .chars()
                    .map(|c| {
                        if matches!(c, '`' | '|' | '\r' | '\n') {
                            ' '
                        } else {
                            c
                        }
                    })
                    .collect();
                cleaned.trim().to_owned()
            } else {
                String::new()
            };
            let pos = if !clean_trait.is_empty() {
                format!("`{clean_trait}`")
            } else {
                String::new()
            };
            let meaning = exp
                .explains
                .iter()
                .map(|e| markdown_text(e).replace(SOFT_BREAK, CELL_BREAK))
                .collect::<Vec<_>>()
                .join("；");
            rows.push(format!("| {pos} | {meaning} |"));
        }
        blocks.push(rows.join("\n"));
    } else {
        let translation = if !view.translation.trim().is_empty() {
            &view.translation
        } else {
            &w.translation
        };
        if !translation.trim().is_empty() {
            blocks.push(markdown_text(translation));
        }
    }

    if !view.associations.is_empty() {
        let joined = view
            .associations
            .iter()
            .map(|a| markdown_text(a))
            .collect::<Vec<_>>()
            .join(" · ");
        blocks.push(quote(&format!("**常用搭配**：{joined}")));
    }

    blocks.extend(example_blocks(&view.examples));
    blocks.extend(note_blocks(&view.notes));

    if let Some(from) = from_text.filter(|s| !s.trim().is_empty()) {
        blocks.push(format!("出自：{}", markdown_text(from)));
    }

    blocks
}

fn sentence_blocks(view: &EntryDisplay, s: &Entry, children: &[&Entry]) -> Vec<String> {
    let mut blocks = Vec::new();

    if let Some(diff) = view.difficulty.filter(|&d| d > 0) {
        let stars = "★".repeat(diff as usize);
        let reason = if !view.difficulty_reason.trim().is_empty() {
            format!(" {}", markdown_text(&view.difficulty_reason))
        } else {
            String::new()
        };
        blocks.push(format!("难度：{stars}{reason}"));
    }

    if !s.text.trim().is_empty() {
        blocks.push(italic(&s.text));
    }

    let translation = if !view.translation.trim().is_empty() {
        &view.translation
    } else {
        &s.translation
    };
    if !translation.trim().is_empty() {
        // 译文不进引用块：跟上面的原句同一个斜体版式、直接跟在下面（用户手测定的，
        // 旧版 `wordbook_format.js` 这里是 `>` 引用）
        blocks.push(italic(translation));
    }

    if let Some(syntax) = &view.syntax_breakdown {
        let main = syntax.main_clause.trim();
        if !main.is_empty() {
            let code = code_text(main);
            let trunk = if main.contains('\r') || main.contains('\n') || code.is_empty() {
                markdown_text(main)
            } else {
                format!("`{code}`")
            };
            blocks.push(format!("**核心句型**：{trunk}"));
        }
        if !syntax.clauses_and_modifiers.trim().is_empty() {
            blocks.push(format!(
                "**修饰成分**：{}",
                markdown_text(&syntax.clauses_and_modifiers)
            ));
        }
    }

    if !view.nuance_note.trim().is_empty() {
        blocks.push(format!(
            "**语境说明**：{}",
            markdown_text(&view.nuance_note)
        ));
    }

    if !view.key_vocabulary.is_empty() {
        let terms = view
            .key_vocabulary
            .iter()
            .map(|v| {
                format!(
                    "`{} ({})`",
                    code_text(&v.word),
                    code_text(&v.meaning_in_context)
                )
            })
            .collect::<Vec<_>>()
            .join(" · ");
        blocks.push(format!("**重点术语**：{terms}"));
    }

    blocks.extend(example_blocks(&view.examples));
    blocks.extend(note_blocks(&view.notes));

    if !children.is_empty() {
        let words = children
            .iter()
            .map(|w| markdown_text(&w.text))
            .collect::<Vec<_>>()
            .join(" · ");
        blocks.push(format!("生词：{words}"));
    }

    blocks
}

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone};
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, html};
    use serde_json::json;

    use super::*;
    use crate::logic::result::Kind;
    use crate::logic::wordbook::Entry;

    fn fixed_now() -> DateTime<Local> {
        let Some(naive) =
            chrono::NaiveDate::from_ymd_opt(2026, 9, 18).and_then(|d| d.and_hms_opt(14, 30, 0))
        else {
            panic!("valid naive date");
        };
        let Some(dt) = Local.from_local_datetime(&naive).earliest() else {
            panic!("valid local date");
        };
        dt
    }

    fn render(markdown: &str) -> (String, String, Vec<Event<'static>>) {
        let mut options = Options::empty();
        options.insert(Options::ENABLE_TABLES);
        let parser = Parser::new_ext(markdown, options);
        let events: Vec<Event<'static>> = parser.into_iter().map(|e| e.into_static()).collect();

        let mut html = String::new();
        html::push_html(&mut html, events.clone().into_iter());

        let mut text = String::new();
        for ev in &events {
            match ev {
                Event::Text(t) | Event::Code(t) => {
                    text.push_str(&t.replace('\u{00a0}', " ").replace('\u{200b}', ""));
                }
                Event::HardBreak | Event::SoftBreak => {
                    text.push('\n');
                }
                Event::End(TagEnd::Paragraph | TagEnd::Heading(..) | TagEnd::Item) => {
                    text.push('\n');
                }
                _ => {}
            }
        }

        (html, text, events)
    }

    #[test]
    fn test_1_escape_rendering() {
        let special = "# 标题、> 引用、- 列表、`code`、*星号*、_下划线_、| 竖线 |、\\反斜线、<b>标签</b>、&amp;、[链接](http://x)、两个空格  和\n\t制表符";
        let multiline_notes = "line1\n\n    indented line\n\tline with tab";

        let detail = json!({
            "schemaVersion": 1,
            "kind": "word",
            "pronunciations": [{"symbol": special}],
            "explanations": [
                {"trait": "n.", "explains": [special]}
            ],
            "associations": [special],
            "examples": [
                {"text": special, "translation": special}
            ],
            "notes": [multiline_notes]
        });

        let entry = Entry {
            id: 1,
            kind: Kind::Word,
            text: "word".into(),
            translation: "".into(),
            detail: Some(detail),
            service: "ai".into(),
            created_at: 100,
        };

        let md = build_markdown(&[entry], fixed_now());
        let (html, text, events) = render(&md);

        // Special characters survive literally in decoded text
        assert!(text.contains("# 标题"));
        assert!(text.contains("> 引用"));
        assert!(text.contains("- 列表"));
        assert!(text.contains("| 竖线 |"));
        assert!(text.contains("\\反斜线"));
        assert!(text.contains("<b>标签</b>"));
        assert!(text.contains("&amp;"));
        assert!(text.contains("[链接](http://x)"));
        assert!(text.contains("两个空格  和\n    制表符"));

        // Multiline notes preserve empty lines and indentation
        assert!(text.contains("line1\n\n    indented line\n    line with tab"));

        // No unwanted HTML structural tags leaked from input fields
        assert!(!html.contains("<h1>标题"));
        assert!(!html.contains("<h2>标题"));
        assert!(!html.contains("<h3>标题"));
        assert!(!html.contains("<h4>标题"));
        assert!(!html.contains("<b>标签</b>"));
        assert!(!html.contains("<a href="));
        assert!(!html.contains("<ul><li>列表"));
        assert!(!html.contains("<ol><li>列表"));
        assert!(html.contains("&lt;b&gt;标签&lt;/b&gt;"));

        // Confirm markdown contains entity replacements
        assert!(md.contains("&#35;"));
        assert!(md.contains("&#62;"));
        assert!(md.contains("&#60;"));
        assert!(md.contains("&#92;"));
        assert!(md.contains("&#160;"));
        assert!(md.contains("&#8203;"));

        // Structural elements we expect: table, blockquote, our own headings
        let has_table = events
            .iter()
            .any(|e| matches!(e, Event::Start(Tag::Table(_))));
        let has_quote = events
            .iter()
            .any(|e| matches!(e, Event::Start(Tag::BlockQuote(_))));
        assert!(has_table, "expected explanation table");
        assert!(has_quote, "expected association blockquote");
    }

    /// 释义里带硬换行时，单元格必须仍然是一行 `| ... |`：`\n` 会当场把表格截断。
    #[test]
    fn test_1b_table_cell_newline_becomes_br() {
        let detail = json!({
            "schemaVersion": 1,
            "kind": "word",
            "explanations": [{"trait": "n.", "explains": ["第一行\n第二行"]}]
        });
        let entry = Entry {
            id: 1,
            kind: Kind::Word,
            text: "word".into(),
            translation: String::new(),
            detail: Some(detail),
            service: "ai".into(),
            created_at: 100,
        };

        let md = build_markdown(&[entry], fixed_now());
        assert!(
            md.contains("| `n.` | 第一行<br>第二行 |"),
            "表格单元格该整行落在一行里：\n{md}"
        );

        let (html, _, events) = render(&md);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, Event::Start(Tag::Table(_)))),
            "表格没被换行截断"
        );
        assert!(html.contains("第一行"), "{html}");
        assert!(html.contains("第二行"), "{html}");
    }

    #[test]
    fn test_2_italic_punctuation_wrapping() {
        let cases = [
            "按照说明操作。",
            ".leading and trailing.",
            "!exclamation!",
            "?question?",
            "(parentheses)",
            "「引申义」",
        ];

        for text in cases {
            let wrapped = italic(text);
            let (html, rendered_text, events) = render(&wrapped);
            let has_emphasis = events
                .iter()
                .any(|e| matches!(e, Event::Start(Tag::Emphasis)));
            assert!(
                has_emphasis,
                "italic wrapping failed to parse as emphasis for: {text}"
            );
            assert!(
                html.contains("<em>"),
                "expected <em> tag in HTML for: {text}"
            );
            assert!(
                rendered_text.contains(text),
                "expected text {text} in rendered text {rendered_text}"
            );
        }
    }

    #[test]
    fn test_3_grouping_and_sorting() {
        let words = vec![
            Entry {
                id: 1,
                kind: Kind::Word,
                text: "爱".into(),
                translation: "love".into(),
                detail: None,
                service: "dict".into(),
                created_at: 10,
            },
            Entry {
                id: 2,
                kind: Kind::Word,
                text: "apple".into(),
                translation: "苹果".into(),
                detail: None,
                service: "dict".into(),
                created_at: 20,
            },
            Entry {
                id: 3,
                kind: Kind::Word,
                text: "阿".into(),
                translation: "ah".into(),
                detail: None,
                service: "dict".into(),
                created_at: 30,
            },
            Entry {
                id: 4,
                kind: Kind::Word,
                text: "banana".into(),
                translation: "香蕉".into(),
                detail: None,
                service: "dict".into(),
                created_at: 40,
            },
            Entry {
                id: 5,
                kind: Kind::Word,
                text: "7-zip".into(),
                translation: "压缩软件".into(),
                detail: None,
                service: "dict".into(),
                created_at: 50,
            },
            Entry {
                id: 6,
                kind: Kind::Word,
                text: "すし".into(),
                translation: "寿司".into(),
                detail: None,
                service: "dict".into(),
                created_at: 60,
            },
            Entry {
                id: 7,
                kind: Kind::Word,
                text: "日本語です".into(),
                translation: "是日语".into(),
                detail: None,
                service: "dict".into(),
                created_at: 70,
            },
        ];

        let sentences = vec![
            Entry {
                id: 10,
                kind: Kind::Sentence,
                text: "EN02 hard.".into(),
                translation: "难状语从句。".into(),
                detail: Some(
                    json!({"kind": "sentence", "translation": "难状语从句。", "category": "EN02", "difficulty": 2, "difficulty_reason": "分词复杂"}),
                ),
                service: "ai".into(),
                created_at: 100,
            },
            Entry {
                id: 11,
                kind: Kind::Sentence,
                text: "EN02 easy old.".into(),
                translation: "易状语旧。".into(),
                detail: Some(
                    json!({"kind": "sentence", "translation": "易状语旧。", "category": "EN02", "difficulty": 1}),
                ),
                service: "ai".into(),
                created_at: 50,
            },
            Entry {
                id: 12,
                kind: Kind::Sentence,
                text: "EN02 easy new.".into(),
                translation: "易状语新。".into(),
                detail: Some(
                    json!({"kind": "sentence", "translation": "易状语新。", "category": "EN02", "difficulty": 1}),
                ),
                service: "ai".into(),
                created_at: 200,
            },
            Entry {
                id: 13,
                kind: Kind::Sentence,
                text: "EN02 unrated.".into(),
                translation: "未评级状语。".into(),
                detail: Some(
                    json!({"kind": "sentence", "translation": "未评级状语。", "category": "EN02"}),
                ),
                service: "ai".into(),
                created_at: 300,
            },
            Entry {
                id: 14,
                kind: Kind::Sentence,
                text: "EN01 sentence.".into(),
                translation: "定语从句。".into(),
                detail: Some(
                    json!({"kind": "sentence", "translation": "定语从句。", "category": "EN01", "difficulty": 3}),
                ),
                service: "ai".into(),
                created_at: 10,
            },
            Entry {
                id: 15,
                kind: Kind::Sentence,
                text: "在数字化转型的背景下，企业需要持续投入。".into(),
                translation: "在数字化转型的背景下，企业需要持续投入。".into(),
                detail: Some(
                    json!({"kind": "sentence", "translation": "在数字化转型的背景下，企业需要持续投入。", "category": "ZH02", "difficulty": 2}),
                ),
                service: "ai".into(),
                created_at: 10,
            },
            Entry {
                id: 16,
                kind: Kind::Sentence,
                text: "Uncategorized sentence.".into(),
                translation: "未分类句子。".into(),
                detail: None,
                service: "google".into(),
                created_at: 5,
            },
        ];

        let mut all_entries = words;
        all_entries.extend(sentences);

        let md = build_markdown(&all_entries, fixed_now());

        // Heading order assertions for words
        let word_a = md.find("### A");
        let word_apple = md.find("#### 📖 **apple**");
        let word_a_chinese_1 = md.find("#### 📖 **阿**");
        let word_a_chinese_2 = md.find("#### 📖 **爱**");
        let word_b = md.find("### B");
        let word_hash = md.find("### &#35;");
        let word_7zip = md.find("#### 📖 **7-zip**");
        let word_sushi = md.find("#### 📖 **すし**");
        let word_nihongo = md.find("#### 📖 **日本語です**");

        let Some(pos_a) = word_a else {
            panic!("missing ### A");
        };
        let Some(pos_apple) = word_apple else {
            panic!("missing apple");
        };
        let Some(pos_a1) = word_a_chinese_1 else {
            panic!("missing 阿");
        };
        let Some(pos_a2) = word_a_chinese_2 else {
            panic!("missing 爱");
        };
        let Some(pos_b) = word_b else {
            panic!("missing ### B");
        };
        let Some(pos_hash) = word_hash else {
            panic!("missing ### &#35;");
        };
        let Some(pos_7zip) = word_7zip else {
            panic!("missing 7-zip");
        };
        let Some(pos_sushi) = word_sushi else {
            panic!("missing すし");
        };
        let Some(pos_nihongo) = word_nihongo else {
            panic!("missing 日本語です");
        };

        assert!(pos_a < pos_apple);
        assert!(pos_apple < pos_a1, "English word before Chinese in group A");
        assert!(pos_a1 < pos_a2, "阿 (a) before 爱 (ai)");
        assert!(pos_a2 < pos_b);
        assert!(pos_b < pos_hash, "group # is last");
        assert!(pos_hash < pos_7zip);
        assert!(pos_7zip < pos_sushi);
        assert!(pos_sushi < pos_nihongo);

        // Sentence category order assertions
        let cat_en01 = md.find("### 英文 · 01 定语从句类");
        let cat_en02 = md.find("### 英文 · 02 状语从句类");
        let cat_zh02 = md.find("### 中文 · 02 长状语阻隔类");
        let cat_uncat = md.find("### 未分类");

        let Some(p_en01) = cat_en01 else {
            panic!("missing EN01");
        };
        let Some(p_en02) = cat_en02 else {
            panic!("missing EN02");
        };
        let Some(p_zh02) = cat_zh02 else {
            panic!("missing ZH02");
        };
        let Some(p_uncat) = cat_uncat else {
            panic!("missing uncat");
        };

        assert!(p_en01 < p_en02);
        assert!(p_en02 < p_zh02);
        assert!(p_zh02 < p_uncat);

        // Within EN02: difficulty ascending (1, 1, 2, unrated=4), ties by created_at desc (200 before 50)
        let s_1 = md.find("#### 1.");
        let s_2 = md.find("#### 2. *EN02 easy new.*");
        let s_3 = md.find("#### 3. *EN02 easy old.*");
        let s_4 = md.find("#### 4. *EN02 hard.*");
        let s_5 = md.find("#### 5. *EN02 unrated.*");
        let s_6 = md.find("#### 6. *在数字化转型的背景下，企业需要持续投入。*");
        let s_7 = md.find("#### 7. *Uncategorized sentence.*");

        let Some(p1) = s_1 else {
            panic!("missing 1.");
        };
        let Some(p2) = s_2 else {
            panic!("missing 2. EN02 easy new");
        };
        let Some(p3) = s_3 else {
            panic!("missing 3. EN02 easy old");
        };
        let Some(p4) = s_4 else {
            panic!("missing 4. EN02 hard");
        };
        let Some(p5) = s_5 else {
            panic!("missing 5. EN02 unrated");
        };
        let Some(p6) = s_6 else {
            panic!("missing 6. ZH02");
        };
        let Some(p7) = s_7 else {
            panic!("missing 7. Uncat");
        };

        assert!(p1 < p2);
        assert!(p2 < p3);
        assert!(p3 < p4);
        assert!(p4 < p5);
        assert!(p5 < p6);
        assert!(p6 < p7);
    }

    #[test]
    fn test_4_summary_truncation() {
        let en_text =
            "Network latency impacts collaboration between teams across different timezones.";
        let zh_text =
            "在数字化转型的时代背景下企业必须不断提升自身的软件开发与运维能力及团队协作水平";

        let entries = vec![
            Entry {
                id: 1,
                kind: Kind::Sentence,
                text: en_text.into(),
                translation: "网络延迟影响跨时区团队协作。".into(),
                detail: None,
                service: "google".into(),
                created_at: 100,
            },
            Entry {
                id: 2,
                kind: Kind::Sentence,
                text: zh_text.into(),
                translation:
                    "在数字化转型的时代背景下企业必须不断提升自身的软件开发与运维能力及团队协作水平"
                        .into(),
                detail: None,
                service: "google".into(),
                created_at: 50,
            },
        ];

        let md = build_markdown(&entries, fixed_now());

        // English cuts at last space <= 35 when > 20: "Network latency impacts" + "..."
        assert!(md.contains("#### 1. *Network latency impacts...*"));

        // Chinese has no spaces, cuts at exactly 35 chars + "..."
        // First 35 chars: "在数字化转型的时代背景下企业必须不断提升自身的软件开发与运维能力及团队"
        let expected_zh =
            "在数字化转型的时代背景下企业必须不断提升自身的软件开发与运维能力及团队...";
        assert!(md.contains(&format!("#### 2. *{expected_zh}*")));
    }

    #[test]
    fn test_5_fallback_paths() {
        // 1. Google plain sentence (detail = None)
        let google_entry = Entry {
            id: 1,
            kind: Kind::Sentence,
            text: "Google plain sentence.".into(),
            translation: "谷歌纯翻译句子。".into(),
            detail: None,
            service: "google".into(),
            created_at: 10,
        };

        // 2. Broken JSON / invalid detail entry (detail = None)
        let broken_entry = Entry {
            id: 2,
            kind: Kind::Sentence,
            text: "Broken detail sentence.".into(),
            translation: "坏数据句子。".into(),
            detail: None,
            service: "ai".into(),
            created_at: 5,
        };

        let md = build_markdown(&[google_entry, broken_entry], fixed_now());

        assert!(md.contains("## 长难句"));
        assert!(md.contains("### 未分类"));
        assert!(md.contains("#### 1. *Google plain sentence.*"));
        assert!(md.contains("*Google plain sentence.*"));
        assert!(
            md.contains(
                "*Google plain sentence.*

*谷歌纯翻译句子。*"
            ),
            "译文紧跟原句、同样的斜体，不进引用块"
        );
        assert!(md.contains("#### 2. *Broken detail sentence.*"));
        assert!(md.contains("*Broken detail sentence.*"));
        assert!(md.contains(
            "*Broken detail sentence.*

*坏数据句子。*"
        ));

        for label in [
            "难度：",
            "核心句型",
            "修饰成分",
            "语境说明",
            "重点术语",
            "例句",
            "补充说明",
            "生词：",
        ] {
            assert!(
                !md.contains(label),
                "unexpected label '{label}' in fallback output"
            );
        }

        // 3. Empty entries
        let empty_md = build_markdown(&[], fixed_now());
        assert!(
            empty_md.starts_with("# 我的生词本\n\n> 导出于 2026-09-18 14:30 · 单词 0 · 长难句 0\n")
        );
        assert!(!empty_md.contains("## 单词"));
        assert!(!empty_md.contains("## 长难句"));
    }

    #[test]
    fn test_6_now_injection() {
        let entries = vec![
            Entry {
                id: 1,
                kind: Kind::Word,
                text: "test".into(),
                translation: "测试".into(),
                detail: None,
                service: "dict".into(),
                created_at: 10,
            },
            Entry {
                id: 2,
                kind: Kind::Sentence,
                text: "Testing now injection.".into(),
                translation: "测试时间注入。".into(),
                detail: None,
                service: "google".into(),
                created_at: 10,
            },
        ];

        let now = fixed_now();
        let md = build_markdown(&entries, now);
        let lines: Vec<&str> = md.lines().collect();

        assert_eq!(lines[0], "# 我的生词本");
        assert_eq!(lines[1], "");
        assert_eq!(lines[2], "> 导出于 2026-09-18 14:30 · 单词 1 · 长难句 1");
    }

    #[test]
    fn test_7_structured_word_and_sentence_blocks() {
        let word_detail = json!({
            "schemaVersion": 1,
            "kind": "word",
            "pronunciations": [{"symbol": "/ˈfɑːloʊ/"}, {"symbol": "/ˈfɒləʊ/"}],
            "explanations": [
                {"trait": "v.", "explains": ["跟随", "听从"]},
                {"trait": "n.", "explains": ["关注"]}
            ],
            "associations": ["follow up", "as follows"],
            "examples": [
                {"text": "Follow the instructions.", "translation": "按照说明操作。"}
            ],
            "notes": ["follow 后可直接接宾语。"]
        });

        let sentence_detail = json!({
            "schemaVersion": 1,
            "kind": "sentence",
            "category": "EN01",
            "difficulty": 2,
            "difficulty_reason": "定语从句后置修饰",
            "translation": "网络延迟影响团队间的有效协作。",
            "syntax_breakdown": {
                "main_clause": "Network latency impacts collaboration",
                "clauses_and_modifiers": "between teams 修饰 collaboration"
            },
            "nuance_note": "impact 这里是及物动词。",
            "key_vocabulary": [
                {"word": "latency", "meaning_in_context": "延迟"}
            ],
            "examples": [
                {"text": "Low latency is critical.", "translation": "低延迟至关重要。"}
            ],
            "notes": ["注意 latency 与 delay 的区别。"]
        });

        let entries = vec![
            Entry {
                id: 1,
                kind: Kind::Word,
                text: "follow".into(),
                translation: "跟随".into(),
                detail: Some(word_detail),
                service: "ai".into(),
                created_at: 100,
            },
            Entry {
                id: 2,
                kind: Kind::Sentence,
                text: "Network latency impacts collaboration between teams.".into(),
                translation: "网络延迟影响协作。".into(),
                detail: Some(sentence_detail),
                service: "ai".into(),
                created_at: 200,
            },
        ];

        let md = build_markdown(&entries, fixed_now());
        let (html, text, events) = render(&md);

        // Word assertions
        assert!(md.contains("`/ˈfɑːloʊ/` · `/ˈfɒləʊ/`"));
        assert!(md.contains("| `v.` | 跟随；听从 |"));
        assert!(md.contains("| `n.` | 关注 |"));
        assert!(md.contains("> **常用搭配**：follow up · as follows"));
        assert!(md.contains("**例句**\n\n1. Follow the instructions.  \n   *按照说明操作。*"));
        assert!(md.contains("**补充说明**\n\nfollow 后可直接接宾语。"));

        // Sentence assertions
        assert!(md.contains("难度：★★ 定语从句后置修饰"));
        assert!(md.contains(
            "*Network latency impacts collaboration between teams.*

*网络延迟影响团队间的有效协作。*"
        ));
        assert!(md.contains("**核心句型**：`Network latency impacts collaboration`"));
        assert!(md.contains("**修饰成分**：between teams 修饰 collaboration"));
        assert!(md.contains("**语境说明**：impact 这里是及物动词。"));
        assert!(md.contains("**重点术语**：`latency (延迟)`"));

        // Table rendered in HTML
        assert!(html.contains("<table>"));
        assert!(html.contains("词性"));
        assert!(html.contains("详细释义"));
        let has_table = events
            .iter()
            .any(|e| matches!(e, Event::Start(Tag::Table(_))));
        let has_th = events
            .iter()
            .any(|e| matches!(e, Event::Start(Tag::TableHead)));
        assert!(has_table);
        assert!(has_th);

        // Rendered text contains all expected content
        assert!(text.contains("跟随；听从"));
        assert!(text.contains("低延迟至关重要。"));
    }
}
