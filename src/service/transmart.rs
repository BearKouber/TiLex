//! 腾讯交互翻译（Transmart）服务（从旧 `services/translate/transmart/index.jsx` 移植）：
//! 免 key 内置接口，固定请求头和 client_key。
//! 返回值：纯译文是 `Value::String`（`trim()` 过）。

use serde_json::{Value, json};

use crate::error::{Error, HttpKind};
use crate::service::http;

const URL: &str = "https://transmart.qq.com/api/imt";
const REFERER: &str = "https://yi.qq.com/zh-CN/index";
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/110.0.0.0 Safari/537.36";
const CLIENT_KEY: &str =
    "browser-chrome-110.0.0-Mac OS-df4bd4c5-a65d-44b2-a40f-42f34f3535f2-1677486696487";

/// TiLex 语言码 → Transmart 语言码。不在表里的就是这个服务不支持。
fn lang(code: &str) -> Option<&'static str> {
    Some(match code {
        "auto" => "auto",
        "zh_cn" => "zh",
        "zh_tw" => "zh-TW",
        "en" => "en",
        "ja" => "ja",
        "ko" => "ko",
        "fr" => "fr",
        "es" => "es",
        "ru" => "ru",
        "de" => "de",
        "it" => "it",
        "tr" => "tr",
        "pt_pt" | "pt_br" => "pt",
        "vi" => "vi",
        "id" => "id",
        "th" => "th",
        "ms" => "ms",
        "ar" => "ar",
        _ => return None,
    })
}

/// `from` / `to` 是 TiLex 语言码（`auto`、`zh_cn` …）。
pub fn translate(text: &str, from: &str, to: &str) -> Result<Value, Error> {
    let from = if from.is_empty() { "auto" } else { from };
    let to = if to.is_empty() { "zh_cn" } else { to };
    let (Some(from), Some(to)) = (lang(from), lang(to)) else {
        return Err(Error::LanguageUnsupported);
    };
    if to == "auto" {
        return Err(Error::LanguageUnsupported);
    }

    let headers = [("Referer", REFERER), ("User-Agent", USER_AGENT)];
    let body = json!({
        "header": {
            "fn": "auto_translation_block",
            "client_key": CLIENT_KEY,
        },
        "type": "plain",
        "model_category": "normal",
        "source": {
            "lang": from,
            "text_block": text,
        },
        "target": {
            "lang": to,
        },
    });

    let data = http::post_json(URL, &headers, &body, http::TIMEOUT_TRANSLATE)?;
    parse_response(&data)
        .map(Value::String)
        .ok_or_else(bad_response)
}

fn bad_response() -> Error {
    Error::Http {
        status: Some(200),
        kind: HttpKind::Format,
    }
}

pub(crate) fn parse_response(data: &Value) -> Option<String> {
    let text = data.get("auto_translation")?.as_str()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_response_extracts_translation() {
        let valid = json!({
            "auto_translation": "  你好世界  "
        });
        assert_eq!(parse_response(&valid), Some("你好世界".to_owned()));
    }

    #[test]
    fn parse_response_handles_invalid_or_empty() {
        let empty_text = json!({
            "auto_translation": "   "
        });
        assert_eq!(parse_response(&empty_text), None);

        let missing = json!({
            "code": 500,
            "message": "fail"
        });
        assert_eq!(parse_response(&missing), None);

        let non_string = json!({
            "auto_translation": 123
        });
        assert_eq!(parse_response(&non_string), None);

        let err = parse_response(&missing)
            .map(Value::String)
            .ok_or_else(bad_response)
            .unwrap_err();
        assert_eq!(err.to_string(), "unexpected response from the server");
    }

    #[test]
    fn language_table() {
        assert_eq!(lang("zh_cn"), Some("zh"));
        assert_eq!(lang("zh_tw"), Some("zh-TW"));
        assert_eq!(lang("pt_pt"), Some("pt"));
        assert_eq!(lang("pt_br"), Some("pt"));
        assert_eq!(lang("auto"), Some("auto"));
        assert_eq!(lang("unsupported"), None);

        assert!(matches!(
            translate("hello", "unsupported", "zh_cn"),
            Err(Error::LanguageUnsupported)
        ));
        assert!(matches!(
            translate("hello", "en", "auto"),
            Err(Error::LanguageUnsupported)
        ));
    }

    #[test]
    #[ignore]
    fn live_translation() {
        let out = translate("Hello world", "auto", "zh_cn").unwrap();
        assert!(out.as_str().is_some_and(|s| !s.is_empty()), "{out}");
    }
}
