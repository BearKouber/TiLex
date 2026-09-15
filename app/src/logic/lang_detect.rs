//! 本地语种识别（design §2.6，D13，lingua 1.8 高精度模式）。
//!
//! 哪些语言被编进来完全由 Cargo.toml 的 feature 决定（`detect-lite` 中日英韩，`detect-full` 22 种），这里不列语言：
//! lingua 的 `Language` 变体随 feature 门控，`from_all_languages()` 拿到的就是这次编译带的。
//! 检测器第一次用时建一次，全程复用。**只许在后台线程调**（取词 worker、翻译调度线程），UI 线程不许（R-5）。

/// 识别出 TiLex 语言码（`zh_cn`、`en` …）；认不出、或没编 lingua 时返回 `None`，调用方回退源语言。
pub fn detect(text: &str) -> Option<&'static str> {
    imp::detect(text)
}

/// 文字是不是已经是目标语言（排除母语用）。先按书写系统粗判，对得上再用 lingua 细分同书写系统的语言；
/// lingua 认不出或没编进来时以书写系统的结论为准。会跑 lingua，只许后台线程调（R-5）。
pub fn is_native_language(text: &str, target: &str) -> bool {
    // 两级：先用书写系统快速否掉（不同文字系统的一定不是目标语言，
    // 这一步不花钱），脚本对得上时才让 lingua 去区分同一书写系统里的
    // 语言 —— 英 / 法 / 德 这种脚本分不开的，只有真检测能分。
    if !is_native_language_for(text, target) {
        return false;
    }
    // 本地识别没编进来（--no-default-features），或者文本太短认不出，
    // 就退回脚本那一层的结论。
    match detect(text) {
        Some(code) => same_base_language(code, target),
        None => true,
    }
}

// zh_cn / zh_tw、pt_pt / pt_br 这种只差地区的算同一种：lingua 本来也只
// 返回大类（Language::Chinese → zh_cn），比到地区没有意义。
fn same_base_language(a: &str, b: &str) -> bool {
    a.split('_').next() == b.split('_').next()
}

// 字符属于哪种书写系统。第一级粗判，不花钱；书写系统不同的直接否掉，不用跑 lingua。
#[derive(PartialEq, Clone, Copy, Debug)]
enum Script {
    Han,
    Kana,
    Hangul,
    Cyrillic,
    Arabic,
    Hebrew,
    Thai,
    Devanagari,
    Latin,
    Other,
}

fn script_of(c: char) -> Script {
    match c {
        '\u{4e00}'..='\u{9fff}' | '\u{3400}'..='\u{4dbf}' => Script::Han,
        '\u{3040}'..='\u{30ff}' => Script::Kana,
        '\u{1100}'..='\u{11ff}' | '\u{ac00}'..='\u{d7af}' => Script::Hangul,
        '\u{0400}'..='\u{04ff}' => Script::Cyrillic,
        '\u{0590}'..='\u{05ff}' => Script::Hebrew,
        '\u{0600}'..='\u{06ff}' => Script::Arabic,
        '\u{0e00}'..='\u{0e7f}' => Script::Thai,
        '\u{0900}'..='\u{097f}' => Script::Devanagari,
        // 其他所有字母字符都视为拉丁字母，包含带变音符号的。
        c if c.is_alphabetic() => Script::Latin,
        _ => Script::Other,
    }
}

// 这一级只看书写系统：拉丁字母语言全在一个桶里（目标英语时法语也算"对得上"），
// 同一书写系统里的语言交给 `is_native_language` 的第二级 lingua 去分。
fn target_script(target: &str) -> Script {
    match target.split('_').next().unwrap_or(target) {
        "zh" => Script::Han,
        "ja" => Script::Kana,
        "ko" => Script::Hangul,
        "ru" | "uk" => Script::Cyrillic,
        "ar" | "fa" => Script::Arabic,
        "he" => Script::Hebrew,
        "th" => Script::Thai,
        "hi" => Script::Devanagari,
        _ => Script::Latin,
    }
}

fn is_native_language_for(text: &str, target: &str) -> bool {
    let want = target_script(target);
    let scripts: Vec<Script> = text
        .chars()
        .map(script_of)
        .filter(|s| *s != Script::Other)
        .collect();
    if scripts.is_empty() {
        return false;
    }
    let has_kana = scripts.contains(&Script::Kana);
    match want {
        // 假名只属于日语，因此只要有假名就能判定，即便文本中大多是汉字。
        Script::Kana => has_kana,
        // 反之亦然：有假名就意味着不是中文，所以绝不能当作"已经是目标语言"而跳过。
        Script::Han if has_kana => false,
        _ => scripts.iter().filter(|s| **s == want).count() * 2 > scripts.len(),
    }
}

#[cfg(feature = "local-detect")]
mod imp {
    use std::sync::OnceLock;

    use lingua::{LanguageDetector, LanguageDetectorBuilder};

    static DETECTOR: OnceLock<LanguageDetector> = OnceLock::new();

    pub fn detect(text: &str) -> Option<&'static str> {
        let detector =
            DETECTOR.get_or_init(|| LanguageDetectorBuilder::from_all_languages().build());
        let lang = detector.detect_language_of(text)?;
        code_of(&lang.iso_code_639_1().to_string())
    }

    /// ISO 639-1 → TiLex 语言码。走字符串而不是 match `Language`：枚举变体随 feature 消失，字符串分支不会。
    fn code_of(iso: &str) -> Option<&'static str> {
        Some(match iso {
            "zh" => "zh_cn",
            "ja" => "ja",
            "en" => "en",
            "ko" => "ko",
            "fr" => "fr",
            "es" => "es",
            "de" => "de",
            "ru" => "ru",
            "it" => "it",
            "pt" => "pt_pt",
            "tr" => "tr",
            "ar" => "ar",
            "vi" => "vi",
            "th" => "th",
            "id" => "id",
            "ms" => "ms",
            "hi" => "hi",
            "mn" => "mn_cy",
            "nb" => "nb_no",
            "nn" => "nn_no",
            "fa" => "fa",
            "uk" => "uk",
            "sv" => "sv",
            "pl" => "pl",
            "nl" => "nl",
            "he" => "he",
            _ => return None,
        })
    }
}

#[cfg(not(feature = "local-detect"))]
mod imp {
    pub fn detect(_text: &str) -> Option<&'static str> {
        None
    }
}

#[cfg(test)]
mod tests {
    // 本地识别是「排除母语」和 AI 检测语种的依据。这四种 lite 和 full 都编进来了。
    #[cfg(feature = "local-detect")]
    #[test]
    fn detects_the_languages_this_app_actually_sees() {
        assert_eq!(
            super::detect("The quick brown fox jumps over the lazy dog"),
            Some("en")
        );
        assert_eq!(super::detect("这是一段中文测试文本"), Some("zh_cn"));
        assert_eq!(
            super::detect("私は学生です、よろしくお願いします"),
            Some("ja")
        );
        assert_eq!(super::detect("안녕하세요 반갑습니다"), Some("ko"));
    }

    // 只在完整构建下跑：detect-lite 根本没编法语 / 俄语模型。
    #[cfg(feature = "detect-full")]
    #[test]
    fn full_build_also_covers_the_latin_and_cyrillic_languages() {
        assert_eq!(
            super::detect("Bonjour tout le monde, comment allez-vous"),
            Some("fr")
        );
        assert_eq!(super::detect("Привет, как дела"), Some("ru"));
    }

    use super::{is_native_language_for, same_base_language};

    #[test]
    fn native_language_skips_only_matching_script() {
        assert!(is_native_language_for("这是一段中文", "zh_cn"));
        assert!(is_native_language_for("中文,带标点!", "zh_cn"));
        assert!(!is_native_language_for("hello world", "zh_cn"));
        // mostly English with one Han char still deserves translating
        assert!(!is_native_language_for("hello world 中", "zh_cn"));
        // punctuation and digits alone must not count as native
        assert!(!is_native_language_for("12345 !!!", "zh_cn"));
    }

    // The whole point of dropping the secondary target language: whatever
    // the target is, text already in it must not get a button.
    #[test]
    fn native_language_covers_every_target_not_just_chinese() {
        assert!(is_native_language_for("hello world", "en"));
        assert!(is_native_language_for("bonjour le monde", "fr"));
        assert!(!is_native_language_for("这是一段中文", "en"));
        assert!(is_native_language_for("こんにちは", "ja"));
        assert!(is_native_language_for("안녕하세요", "ko"));
        assert!(is_native_language_for("привет мир", "ru"));
        assert!(!is_native_language_for("hello world", "ru"));
    }

    // Kana is the only thing separating Japanese from Chinese cheaply.
    #[test]
    fn kana_decides_between_japanese_and_chinese() {
        // kanji-heavy Japanese still has kana in it
        assert!(is_native_language_for("私は学生です", "ja"));
        // ...and that same kana must stop it counting as Chinese
        assert!(!is_native_language_for("私は学生です", "zh_cn"));
        // pure Han with a Japanese target is genuinely ambiguous; we do not
        // skip it, so the user still gets a button
        assert!(!is_native_language_for("東京", "ja"));
    }

    // Ceiling of the *script* layer alone: Latin-script languages are one
    // bucket. is_native_language() covers this by asking lingua after the
    // script check passes - this test pins the cheap layer, not the result.
    #[test]
    fn latin_targets_cannot_tell_latin_languages_apart() {
        assert!(is_native_language_for("bonjour le monde", "en"));
    }

    #[test]
    fn base_language_ignores_the_region_suffix() {
        assert!(same_base_language("zh_cn", "zh_tw"));
        assert!(same_base_language("pt_pt", "pt_br"));
        assert!(same_base_language("en", "en"));
        assert!(!same_base_language("en", "de"));
        assert!(!same_base_language("zh_cn", "ja"));
    }

    // 测第二级（lingua）真的起作用。
    #[cfg(feature = "local-detect")]
    #[test]
    fn lingua_disambiguates_languages_sharing_the_same_script() {
        let fr = "Je voudrais réserver une table pour deux personnes ce soir";
        assert!(!super::is_native_language(fr, "en"));
        assert!(super::is_native_language(fr, "fr"));
        let en = "Could I please reserve a table for two people this evening";
        assert!(super::is_native_language(en, "en"));
    }
}
