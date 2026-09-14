//! 源码规则检查（design §4 里标 [CI] 的部分）。只读文件，不编译被检查的代码。
//!
//! 每个 `.rs` 文件从 `#[cfg(test)]` 紧跟 `mod tests` 那一行起到文件末尾不检查（R-9 约定测试模块放末尾）。
//! 注释和字符串字面量里的内容不算代码（`// ignore:` 规则除外，它看的就是注释）。
//! 唯一的豁免：`src/lib.rs` 里 `mod slint_ui` 的模块级 `#![allow(...)]`（包 slint 生成代码）。
#![allow(
    clippy::unwrap_used,
    reason = "clippy.toml 的 allow-unwrap-in-tests 管不到 #[test] 外的辅助函数；读不到源码就该让测试失败"
)]

use std::fs;
use std::path::{Path, PathBuf};

struct Source {
    /// 相对 `app/` 的路径，统一用 `/`。
    rel: String,
    /// 原始行（含注释），测试模块之前的部分。
    raw: Vec<String>,
    /// 去掉注释和字符串内容后的代码行，与 `raw` 一一对应。
    code: Vec<String>,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn walk(dir: &Path, ext: &str, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, ext, out);
        } else if path.extension().is_some_and(|e| e == ext) {
            out.push(path);
        }
    }
}

fn sources() -> Vec<Source> {
    let mut files = Vec::new();
    walk(&root().join("src"), "rs", &mut files);
    assert!(!files.is_empty());
    files
        .into_iter()
        .map(|path| {
            let text = fs::read_to_string(&path).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            let end = lines
                .windows(2)
                .position(|w| {
                    w[0].trim() == "#[cfg(test)]" && w[1].trim_start().starts_with("mod tests")
                })
                .unwrap_or(lines.len());
            let raw: Vec<String> = lines[..end].iter().map(|l| l.to_string()).collect();
            let code = raw.iter().map(|l| code_of(l)).collect();
            let rel = path
                .strip_prefix(root())
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            Source { rel, raw, code }
        })
        .collect()
}

/// 去掉 `//` 注释和字符串字面量的内容（保留引号）。块注释、原始字符串按普通字符处理——本项目不用。
fn code_of(line: &str) -> String {
    let mut out = String::new();
    let mut chars = line.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            match c {
                '\\' => {
                    chars.next();
                }
                '"' => {
                    in_str = false;
                    out.push('"');
                }
                _ => {}
            }
        } else if c == '"' {
            in_str = true;
            out.push('"');
        } else if c == '/' && chars.peek() == Some(&'/') {
            break;
        } else if c == '\'' {
            // 字符字面量 '"' / '\"' 不能当成字符串开头，整个跳过；生命周期 'a 原样保留。
            let ahead: String = chars.clone().take(3).collect();
            let skip = if ahead.starts_with("\"'") {
                2
            } else if ahead.starts_with("\\\"'") {
                3
            } else {
                0
            };
            for _ in 0..skip {
                chars.next();
            }
            out.push(c);
        } else {
            out.push(c);
        }
    }
    out
}

/// 代码里引用到的 `crate::` 顶层模块及其字节位置，包括 `use crate::{a::b, c}` 分组里的
/// （rustfmt 会把长分组拆成多行，所以按整份文件找，不按行）。
fn crate_modules(code: &str) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    for (at, _) in code.match_indices("crate::") {
        let rest = &code[at + "crate::".len()..];
        let Some(group) = rest.strip_prefix('{') else {
            found.push((at, ident(rest)));
            continue;
        };
        // 按第一层逗号切开分组，每项取开头的标识符。
        let mut items = vec![String::new()];
        let mut depth = 0;
        for c in group.chars() {
            match c {
                '{' => depth += 1,
                '}' if depth == 0 => break,
                '}' => depth -= 1,
                ',' if depth == 0 => {
                    items.push(String::new());
                    continue;
                }
                _ => {}
            }
            if let Some(last) = items.last_mut() {
                last.push(c);
            }
        }
        found.extend(items.iter().map(|item| (at, ident(item))));
    }
    found.retain(|(_, m)| !m.is_empty());
    found
}

fn ident(s: &str) -> String {
    s.trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

fn report(rule: &str, violations: Vec<String>) {
    assert!(
        violations.is_empty(),
        "{rule}:\n  {}",
        violations.join("\n  ")
    );
}

/// 在 `src` 下所有文件里找 `needle`（按代码行），`allowed` 里的文件除外。
fn only_in(needle: &str, allowed: &[&str]) -> Vec<String> {
    let mut hits = Vec::new();
    for s in sources() {
        if allowed.contains(&s.rel.as_str()) {
            continue;
        }
        for (i, line) in s.code.iter().enumerate() {
            if line.contains(needle) {
                hits.push(format!("{}:{}: {}", s.rel, i + 1, s.raw[i].trim()));
            }
        }
    }
    hits
}

#[test]
fn layers_only_depend_downwards() {
    // design §1.2
    let forbidden: [(&str, &[&str]); 4] = [
        ("src/ui/", &["service"]),
        ("src/logic/", &["ui"]),
        ("src/service/", &["ui", "logic"]),
        ("src/platform/", &["ui", "logic", "service"]),
    ];
    let mut violations = Vec::new();
    for s in sources() {
        let Some((_, banned)) = forbidden.iter().find(|(dir, _)| s.rel.starts_with(dir)) else {
            continue;
        };
        let text = s.code.join("\n");
        for (at, m) in crate_modules(&text) {
            if banned.contains(&m.as_str()) {
                let line = text[..at].matches('\n').count() + 1;
                violations.push(format!("{}:{line}: uses crate::{m}", s.rel));
            }
        }
    }
    report("layer rule (design §1.2)", violations);
}

#[test]
fn ureq_only_in_http() {
    report(
        "R-4: ureq only in src/service/http.rs",
        only_in("ureq", &["src/service/http.rs"]),
    );
}

#[test]
fn command_new_only_in_process() {
    report(
        "R-4: Command::new only in src/platform/process.rs",
        only_in("Command::new", &["src/platform/process.rs"]),
    );
}

/// 每个 `cfg(...)` / `cfg!(...)` / `cfg_attr(...)` 的起始位置和括号里的内容。
/// 按整份文件找、括号配对：rustfmt 会把长的 `cfg(any(...))` 拆成多行。
fn cfg_args(code: &str) -> Vec<(usize, &str)> {
    let mut found = Vec::new();
    for (at, _) in code.match_indices("cfg") {
        let rest = &code[at + "cfg".len()..];
        let rest = rest
            .strip_prefix("_attr")
            .or_else(|| rest.strip_prefix('!'))
            .unwrap_or(rest);
        let Some(inner) = rest.strip_prefix('(') else {
            continue;
        };
        let mut depth = 1;
        let end = inner
            .char_indices()
            .find(|&(_, c)| {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                depth == 0
            })
            .map_or(inner.len(), |(i, _)| i);
        found.push((at, &inner[..end]));
    }
    found
}

#[test]
fn platform_cfg_only_under_platform() {
    // R-6。按词匹配：`windows_subsystem = "windows"` 的字符串内容已被 code_of 去掉，词也不同，不会误报。
    let platform_words = [
        "target_os",
        "target_family",
        "target_vendor",
        "windows",
        "unix",
        "macos",
    ];
    let mut violations = Vec::new();
    for s in sources() {
        if s.rel.starts_with("src/platform/") {
            continue;
        }
        let text = s.code.join("\n");
        for (at, args) in cfg_args(&text) {
            if args
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|w| platform_words.contains(&w))
            {
                let line = text[..at].matches('\n').count();
                violations.push(format!("{}:{}: {}", s.rel, line + 1, s.raw[line].trim()));
            }
        }
    }
    report("R-6: platform cfg only under src/platform/", violations);
}

#[test]
fn ignored_results_are_explained() {
    // R-3：`let _ =`、`_ = ...;` 和语句级 `.ok();` 同一行或上一行要有 `// ignore: <原因>`（原因不能空）。
    let reason = |raw: &str| {
        raw.split_once("// ignore:")
            .is_some_and(|(_, why)| !why.trim().is_empty())
    };
    let mut violations = Vec::new();
    for s in sources() {
        for (i, line) in s.code.iter().enumerate() {
            let swallows = line.contains("let _ =")
                || line.trim_start().starts_with("_ = ") // 带空格：排除 match 的 `_ =>`
                || line.trim_end().ends_with(".ok();");
            if !swallows {
                continue;
            }
            let here = reason(&s.raw[i]);
            let above = i > 0
                && s.raw[i - 1].trim_start().starts_with("// ignore:")
                && reason(&s.raw[i - 1]);
            if !here && !above {
                violations.push(format!("{}:{}: {}", s.rel, i + 1, s.raw[i].trim()));
            }
        }
    }
    report(
        "R-3: swallowed result needs `// ignore: <reason>`",
        violations,
    );
}

#[test]
fn lint_allows_are_justified() {
    // R-1 / R-2：放开这些 lint（`allow` 或 `expect` 属性）必须写 reason；模块级放开只许 slint_ui 一处。
    // `restriction` 是这几条所在的 lint 组，放开整组等于全放开。
    let lints = [
        "unwrap_used",
        "expect_used",
        "print_stdout",
        "print_stderr",
        "dbg_macro",
        "todo",
        "restriction",
    ];
    let mut violations = Vec::new();
    for s in sources() {
        let text = s.code.join("\n");
        let mut starts: Vec<usize> = text.match_indices("allow(").map(|(i, _)| i).collect();
        // `#[expect(...)]` 属性；`.expect(` 方法调用前面是点或标识符，排除。
        starts.extend(text.match_indices("expect(").map(|(i, _)| i).filter(|&i| {
            !text[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c == '.' || c.is_alphanumeric() || c == '_')
        }));
        for i in starts {
            let before = text[..i].trim_end();
            let module_level = before.ends_with("#![");
            let attr = &text[i..text[i..].find(")]").map_or(text.len(), |e| i + e)];
            let touches = lints.iter().any(|l| attr.contains(&format!("clippy::{l}")));
            if touches && !attr.contains("reason") {
                violations.push(format!("{}: allow without reason: {attr}", s.rel));
            }
            let in_slint_ui = s.rel == "src/lib.rs" && before.contains("mod slint_ui");
            if touches && module_level && !in_slint_ui {
                violations.push(format!(
                    "{}: module-wide allow outside slint_ui: {attr}",
                    s.rel
                ));
            }
        }
    }
    report("R-1/R-2 lint allows", violations);
}

#[test]
fn manifest_keeps_lints_denied() {
    // R-1：这几条 deny 是 R-2 的真正执行者，改成 warn/allow 就等于整条规则失效。
    let manifest = fs::read_to_string(root().join("Cargo.toml")).unwrap();
    for lint in [
        "unwrap_used",
        "expect_used",
        "print_stdout",
        "print_stderr",
        "dbg_macro",
        "todo",
        "unsafe_op_in_unsafe_fn",
    ] {
        assert!(
            manifest.contains(&format!("{lint} = \"deny\"")),
            "Cargo.toml must keep {lint} = \"deny\""
        );
    }
}

#[test]
fn no_unimplemented() {
    // §4.2：没实现的平台函数返回 Err(Error::Unsupported)，不许 unimplemented!()。
    report("§4.2: no unimplemented!()", only_in("unimplemented!", &[]));
}

#[test]
fn ui_does_no_blocking_work() {
    // R-5：界面层不 sleep、不碰 SQLite / HTTP / 服务层 / 语种识别。
    let banned = [
        "sleep", // 包括 `use std::thread::sleep;` 之后的裸 `sleep(..)`
        "rusqlite",
        "ureq",
        "crate::service",
        "lang_detect::detect",
    ];
    let mut violations = Vec::new();
    for s in sources() {
        if !s.rel.starts_with("src/ui/") {
            continue;
        }
        for (i, line) in s.code.iter().enumerate() {
            if let Some(b) = banned.iter().find(|b| line.contains(*b)) {
                violations.push(format!("{}:{}: {b}", s.rel, i + 1));
            }
        }
    }
    report("R-5: src/ui/ must not block", violations);
}

#[test]
fn no_gpu_renderer() {
    // D11：只编软件渲染器。
    let manifest = fs::read_to_string(root().join("Cargo.toml")).unwrap();
    for banned in ["renderer-femtovg", "renderer-skia"] {
        assert!(
            !manifest.contains(banned),
            "Cargo.toml must not enable {banned}"
        );
    }
    // slint 的默认 feature 里就有 renderer-femtovg 和 accessibility：去掉这句 GPU 渲染器就悄悄回来了。
    let slint = manifest
        .lines()
        .find(|l| l.trim_start().starts_with("slint = "))
        .unwrap();
    assert!(
        slint.contains("default-features = false"),
        "slint dependency must keep default-features = false: {slint}"
    );
}

/// 读一个双引号字符串字面量的原文（不反转义），返回内容和剩余部分。
fn quoted(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start().strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                out.push(c);
                out.push(chars.next()?.1);
            }
            '"' => return Some((out, &s[i + 1..])),
            _ => out.push(c),
        }
    }
    None
}

/// `.po` 里的 msgid → msgstr（支持续行）。
fn po_entries(text: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let (mut id, mut value, mut in_str) = (None::<String>, String::new(), false);
    for line in text.lines().map(str::trim) {
        if let Some(rest) = line.strip_prefix("msgid ") {
            if let Some(prev) = id.take() {
                entries.push((prev, std::mem::take(&mut value)));
            }
            id = quoted(rest).map(|(s, _)| s);
            in_str = false;
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            value = quoted(rest).map(|(s, _)| s).unwrap_or_default();
            in_str = true;
        } else if line.starts_with('"') {
            let part = quoted(line).map(|(s, _)| s).unwrap_or_default();
            match (&mut id, in_str) {
                (_, true) => value.push_str(&part),
                (Some(id), false) => id.push_str(&part),
                (None, false) => {}
            }
        }
    }
    if let Some(prev) = id {
        entries.push((prev, value));
    }
    entries
}

#[test]
fn every_tr_string_is_translated() {
    // design §2.9：每条 @tr("...") 在 tilex.po 里都有非空 msgstr。
    let po = fs::read_to_string(root().join("ui/i18n/zh_CN/LC_MESSAGES/tilex.po")).unwrap();
    let entries = po_entries(&po);
    let mut files = Vec::new();
    walk(&root().join("ui"), "slint", &mut files);
    let mut missing = Vec::new();
    let mut count = 0;
    for path in files {
        let text = fs::read_to_string(&path).unwrap();
        let mut rest = text.as_str();
        while let Some(i) = rest.find("@tr(") {
            rest = &rest[i + "@tr(".len()..];
            let Some((mut msgid, after)) = quoted(rest) else {
                continue;
            };
            // @tr("上下文" => "文字")：取箭头后面那个。
            if let Some(after) = after.trim_start().strip_prefix("=>")
                && let Some((text, _)) = quoted(after)
            {
                msgid = text;
            }
            count += 1;
            let ok = entries.iter().any(|(id, s)| *id == msgid && !s.is_empty());
            if !ok {
                missing.push(format!(
                    "{}: {msgid:?}",
                    path.file_name().unwrap().to_string_lossy()
                ));
            }
        }
    }
    assert!(count > 0, "no @tr found — did the ui/ layout change?");
    report("§2.9: @tr without zh_CN msgstr", missing);
}

#[test]
fn checker_self_test() {
    assert_eq!(code_of(r#"let a = "// x"; // c"#), r#"let a = ""; "#);
    assert_eq!(
        code_of(r#"windows_subsystem = "windows")]"#),
        r#"windows_subsystem = "")]"#
    );
    assert_eq!(code_of(r#"let q = '"'; x // c"#), "let q = '; x ");
    let names = |code: &str| {
        crate_modules(code)
            .into_iter()
            .map(|(_, m)| m)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names("use crate::{\n    error::Error,\n    logic::{config, x},\n};"),
        ["error", "logic"]
    );
    assert_eq!(
        names("crate::service::google::t(); crate::ui::x"),
        ["service", "ui"]
    );
    let cfgs = |code: &str| {
        cfg_args(code)
            .into_iter()
            .map(|(_, a)| a.split_whitespace().collect::<String>())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        cfgs("#[cfg(any(\n    target_os = \"\",\n    windows\n))]\nfn f() {}"),
        ["any(target_os=\"\",windows)"]
    );
    assert_eq!(
        cfgs("if cfg!(unix) {} #![cfg_attr(not(x), y)] let config = 1;"),
        ["unix", "not(x),y"]
    );
    let po = "msgid \"\"\nmsgstr \"\"\n\"H: v\\n\"\n\nmsgid \"A \\\"q\\\"\"\nmsgstr \"\"\n\"甲\"\n";
    assert_eq!(
        po_entries(po)[1],
        ("A \\\"q\\\"".to_string(), "甲".to_string())
    );
}
