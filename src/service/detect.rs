//! 在线语种检测引擎（niutrans / baidu / google）。
//!
//! 供 `logic::translate::dispatch` 分发。只许在后台线程调，超时用 `TIMEOUT_DETECT`（5 秒）。
//! 检测是建议性的：认不出、请求失败或引擎不认识一律返回 `None`，调用方回退源语言，不阻断翻译。

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::service::http;

const NIUTRANS_URL: &str = "https://test.niutrans.com/NiuTransServer/language";
const BAIDU_URL: &str = "https://fanyi.baidu.com/langdetect";
const GOOGLE_URL: &str = "https://translate.google.com/translate_a/single";

/// 按引擎名在线检测语种，返回 TiLex 语言码。
/// 认不出、请求失败、或 engine 不认识均返回 `None`。
pub fn detect(engine: &str, text: &str) -> Option<&'static str> {
    if text.trim().is_empty() {
        return None;
    }
    match engine {
        "niutrans" => detect_niutrans(text),
        "baidu" => detect_baidu(text),
        "google" => detect_google(text),
        _ => None,
    }
}

fn detect_niutrans(text: &str) -> Option<&'static str> {
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .to_string();
    let query = [("src_text", text), ("source", "text"), ("time", &time)];
    let headers = [("content-type", "application/json")];
    match http::get(NIUTRANS_URL, &query, &headers, http::TIMEOUT_DETECT) {
        Ok(data) => parse_niutrans(&data),
        Err(e) => {
            log::warn!("detect niutrans failed: {e}");
            None
        }
    }
}

fn detect_baidu(text: &str) -> Option<&'static str> {
    let form = [("query", text)];
    match http::post_form(BAIDU_URL, &[], &form, http::TIMEOUT_DETECT) {
        Ok(data) => parse_baidu(&data),
        Err(e) => {
            log::warn!("detect baidu failed: {e}");
            None
        }
    }
}

fn detect_google(text: &str) -> Option<&'static str> {
    let query: [(&str, &str); 21] = [
        ("dt", "at"),
        ("dt", "bd"),
        ("dt", "ex"),
        ("dt", "ld"),
        ("dt", "md"),
        ("dt", "qca"),
        ("dt", "rw"),
        ("dt", "rm"),
        ("dt", "ss"),
        ("dt", "t"),
        ("client", "gtx"),
        ("sl", "auto"),
        ("tl", "zh-CN"),
        ("hl", "zh-CN"),
        ("ie", "UTF-8"),
        ("oe", "UTF-8"),
        ("otf", "1"),
        ("ssel", "0"),
        ("tsel", "0"),
        ("kc", "7"),
        ("q", text),
    ];
    let headers = [("content-type", "application/json")];
    match http::get(GOOGLE_URL, &query, &headers, http::TIMEOUT_DETECT) {
        Ok(data) => parse_google(&data),
        Err(e) => {
            log::warn!("detect google failed: {e}");
            None
        }
    }
}

fn parse_niutrans(data: &Value) -> Option<&'static str> {
    let code = data.get("language")?.as_str()?;
    niutrans_lang(code)
}

fn parse_baidu(data: &Value) -> Option<&'static str> {
    let code = data.get("lan")?.as_str()?;
    baidu_lang(code)
}

fn parse_google(data: &Value) -> Option<&'static str> {
    let code = data.get(2)?.as_str()?;
    google_lang(code)
}

fn niutrans_lang(code: &str) -> Option<&'static str> {
    Some(match code {
        "zh" => "zh_cn",
        "cht" => "zh_cn",
        "en" => "en",
        "ja" => "ja",
        "ko" => "ko",
        "fr" => "fr",
        "es" => "es",
        "ru" => "ru",
        "de" => "de",
        "it" => "it",
        "tr" => "tr",
        "pt" => "pt_pt",
        "vi" => "vi",
        "id" => "id",
        "th" => "th",
        "ms" => "ms",
        "ar" => "ar",
        "hi" => "hi",
        "mn" => "mn_cy",
        "mo" => "mn_mo",
        "km" => "km",
        "nb" => "nb_no",
        "nn" => "nn_no",
        "fa" => "fa",
        "uk" => "uk",
        _ => return None,
    })
}

fn baidu_lang(code: &str) -> Option<&'static str> {
    Some(match code {
        "zh" => "zh_cn",
        "cht" => "zh_tw",
        "en" => "en",
        "jp" => "ja",
        "kor" => "ko",
        "fra" => "fr",
        "spa" => "es",
        "ru" => "ru",
        "de" => "de",
        "it" => "it",
        "tr" => "tr",
        "pt" => "pt_pt",
        "vie" => "vi",
        "id" => "id",
        "th" => "th",
        "may" => "ms",
        "ar" => "ar",
        "hi" => "hi",
        "nob" => "nb_no",
        "nno" => "nn_no",
        "per" => "fa",
        "ukr" => "uk",
        _ => return None,
    })
}

fn google_lang(code: &str) -> Option<&'static str> {
    Some(match code {
        "zh-CN" => "zh_cn",
        "zh-TW" => "zh_tw",
        "ja" => "ja",
        "en" => "en",
        "ko" => "ko",
        "fr" => "fr",
        "es" => "es",
        "ru" => "ru",
        "de" => "de",
        "it" => "it",
        "tr" => "tr",
        "pt" => "pt_pt",
        "vi" => "vi",
        "id" => "id",
        "th" => "th",
        "ms" => "ms",
        "ar" => "ar",
        "hi" => "hi",
        "mn" => "mn_cy",
        "km" => "km",
        "fa" => "fa",
        "no" => "nb_no",
        "uk" => "uk",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_niutrans_success() {
        let sample_zh = json!({ "language": "zh" });
        assert_eq!(parse_niutrans(&sample_zh), Some("zh_cn"));

        let sample_en = json!({ "language": "en" });
        assert_eq!(parse_niutrans(&sample_en), Some("en"));
    }

    #[test]
    fn parse_niutrans_invalid_or_missing() {
        assert_eq!(parse_niutrans(&json!({})), None);
        assert_eq!(parse_niutrans(&json!({ "language": 123 })), None);
        assert_eq!(parse_niutrans(&json!({ "language": "unknown_code" })), None);
        assert_eq!(parse_niutrans(&json!(null)), None);
    }

    #[test]
    fn parse_baidu_success() {
        let sample_zh = json!({ "error": 0, "msg": "success", "lan": "zh" });
        assert_eq!(parse_baidu(&sample_zh), Some("zh_cn"));

        let sample_en = json!({ "error": 0, "msg": "success", "lan": "en" });
        assert_eq!(parse_baidu(&sample_en), Some("en"));
    }

    #[test]
    fn parse_baidu_invalid_or_missing() {
        assert_eq!(parse_baidu(&json!({})), None);
        assert_eq!(parse_baidu(&json!({ "lan": 123 })), None);
        assert_eq!(parse_baidu(&json!({ "lan": "unknown_code" })), None);
        assert_eq!(parse_baidu(&json!(null)), None);
    }

    #[test]
    fn parse_google_success() {
        let sample_zh = json!([[["Hello", "你好", null, null, 1]], null, "zh-CN"]);
        assert_eq!(parse_google(&sample_zh), Some("zh_cn"));

        let sample_en = json!([[["Hi", "嗨", null, null, 1]], null, "en"]);
        assert_eq!(parse_google(&sample_en), Some("en"));
    }

    #[test]
    fn parse_google_invalid_or_missing() {
        assert_eq!(parse_google(&json!([])), None);
        assert_eq!(parse_google(&json!(["only_one"])), None);
        assert_eq!(parse_google(&json!([null, null, 123])), None);
        assert_eq!(parse_google(&json!([null, null, "unknown_code"])), None);
        assert_eq!(parse_google(&json!({})), None);
    }

    #[test]
    fn language_table_special_mappings() {
        // 百度易错项
        assert_eq!(baidu_lang("jp"), Some("ja"));
        assert_eq!(baidu_lang("kor"), Some("ko"));
        assert_eq!(baidu_lang("cht"), Some("zh_tw"));
        assert_eq!(baidu_lang("nob"), Some("nb_no"));
        assert_eq!(baidu_lang("nno"), Some("nn_no"));
        assert_eq!(baidu_lang("per"), Some("fa"));
        assert_eq!(baidu_lang("ukr"), Some("uk"));
        assert_eq!(baidu_lang("unknown"), None);

        // niutrans 易错项（cht 映射到 zh_cn）
        assert_eq!(niutrans_lang("cht"), Some("zh_cn"));
        assert_eq!(niutrans_lang("mo"), Some("mn_mo"));
        assert_eq!(niutrans_lang("mn"), Some("mn_cy"));
        assert_eq!(niutrans_lang("nb"), Some("nb_no"));
        assert_eq!(niutrans_lang("nn"), Some("nn_no"));
        assert_eq!(niutrans_lang("unknown"), None);

        // google 易错项
        assert_eq!(google_lang("zh-CN"), Some("zh_cn"));
        assert_eq!(google_lang("zh-TW"), Some("zh_tw"));
        assert_eq!(google_lang("no"), Some("nb_no"));
        assert_eq!(google_lang("unknown"), None);
    }

    #[test]
    fn unknown_engine_or_empty_text_returns_none() {
        assert_eq!(detect("unknown", "hello"), None);
        assert_eq!(detect("local", "hello"), None);
        assert_eq!(detect("yandex", "hello"), None);
        assert_eq!(detect("tencent", "hello"), None);
        assert_eq!(detect("", "hello"), None);
        assert_eq!(detect("google", ""), None);
        assert_eq!(detect("google", "   "), None);
    }

    #[test]
    #[ignore = "requires network connection"]
    fn live_detect_niutrans() {
        let result = detect("niutrans", "Hello world");
        assert_eq!(result, Some("en"));
    }

    #[test]
    #[ignore = "requires network connection"]
    fn live_detect_baidu() {
        let result = detect("baidu", "你好世界");
        assert_eq!(result, Some("zh_cn"));
    }

    #[test]
    #[ignore = "requires network connection"]
    fn live_detect_google() {
        let result = detect("google", "こんにちは世界");
        assert_eq!(result, Some("ja"));
    }
}
