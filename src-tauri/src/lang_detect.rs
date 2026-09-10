// 本地语种识别（lingua）。2026-09-08 起默认编译。
//
// **哪些语言被编进来完全由 Cargo.toml 的 feature 决定，这个文件不列语言。**
// lingua 的 Language 枚举变体本身是 feature 门控的，所以 from_all_languages()
// 拿到的就是「这次编译带了哪些」，加减语言只改 Cargo.toml 一处。
//
//   cargo build                          → detect-lite：中日英韩，模型约 1.7 MB
//   cargo build --features detect-full   → 22 种语言，模型 24 MB（发布版用这个）
//   cargo build --no-default-features    → 完全不编 lingua，退回 pop_button 的脚本判断
//
// 门控写在这些函数上而不是 main.rs：tauri::generate_handler![] 的列表按路径展开，
// 里面放不了 #[cfg] 属性，所以 main.rs 那几处一行都不用动。

#[cfg(not(feature = "local-detect"))]
pub fn init_lang_detect() {}

#[cfg(not(feature = "local-detect"))]
pub fn detect_code(_text: &str) -> Option<&'static str> {
    None
}

#[cfg(not(feature = "local-detect"))]
#[tauri::command]
pub fn lang_detect(_text: &str) -> Result<&'static str, ()> {
    Err(())
}

#[cfg(feature = "local-detect")]
mod local {
    use lingua::{LanguageDetector, LanguageDetectorBuilder};
    use std::sync::OnceLock;

    // 一次建好，全程复用。原来的实现在每次调用里 build() 一遍，那才是
    // 「lingua 太重」的真正来源 —— 重的是构造，不是识别。
    static DETECTOR: OnceLock<LanguageDetector> = OnceLock::new();

    fn detector() -> &'static LanguageDetector {
        DETECTOR.get_or_init(|| LanguageDetectorBuilder::from_all_languages().build())
    }

    // ISO 639-1 → TiLex 的语言码（utils/language.ts 那套）。
    //
    // 走字符串而不是 match Language 枚举，是为了让这个 match 和 feature 解耦：
    // 枚举变体会随 feature 消失，字符串分支不会，所以 Cargo 里加减语言不用回来改这里。
    // TiLex 语言表里有、但当前 feature 没编进来的，只是走不到而已。
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

    // 启动时预热：把构造和第一次识别的模型加载都挪到这儿，
    // 免得用户第一次划词等一下。
    pub fn init() {
        let _ = detector().detect_language_of("Hello language");
    }

    pub fn detect(text: &str) -> Option<&'static str> {
        let lang = detector().detect_language_of(text)?;
        code_of(&lang.iso_code_639_1().to_string())
    }
}

#[cfg(feature = "local-detect")]
pub fn init_lang_detect() {
    local::init();
}

// 划词按钮那条路（pop_button.rs）直接调这个：同步、本地、无网络。
// 识别不出来返回 None，调用方自己决定怎么退。
#[cfg(feature = "local-detect")]
pub fn detect_code(text: &str) -> Option<&'static str> {
    local::detect(text)
}

#[cfg(feature = "local-detect")]
#[tauri::command]
pub fn lang_detect(text: &str) -> Result<&'static str, ()> {
    // 前端那边把 'en' 当兜底值用了很久，保持一致。
    Ok(local::detect(text).unwrap_or("en"))
}

#[cfg(all(test, feature = "local-detect"))]
mod tests {
    use super::detect_code;

    // 本地识别是划词按钮「要不要弹」的判据，认错就是弹错。
    // 这四种在 detect-lite 和 detect-full 里都编进来了，两种构建都该过。
    #[test]
    fn detects_the_languages_this_app_actually_sees() {
        assert_eq!(
            detect_code("The quick brown fox jumps over the lazy dog"),
            Some("en")
        );
        assert_eq!(detect_code("这是一段中文测试文本"), Some("zh_cn"));
        assert_eq!(detect_code("私は学生です、よろしくお願いします"), Some("ja"));
        assert_eq!(detect_code("안녕하세요 반갑습니다"), Some("ko"));
    }

    // 只在完整构建下跑：detect-lite 根本没编法语 / 俄语模型，认不出来才是对的。
    #[cfg(feature = "detect-full")]
    #[test]
    fn full_build_also_covers_the_latin_and_cyrillic_languages() {
        assert_eq!(
            detect_code("Bonjour tout le monde, comment allez-vous"),
            Some("fr")
        );
        assert_eq!(detect_code("Привет, как дела"), Some("ru"));
    }
}
