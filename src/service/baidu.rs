//! 百度翻译服务（从旧 `services/translate/baidu/index.jsx` 移植）：
//! GET 请求，appid + text + salt + secret 的 MD5 签名。
//! 返回值：纯译文是 `Value::String`（`trim()` 过）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, HttpKind};
use crate::service::http;

const URL: &str = "https://fanyi-api.baidu.com/api/trans/vip/translate";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub appid: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub secret: String,
}

/// TiLex 语言码 → 百度语言码。不在表里的就是这个服务不支持。
fn lang(code: &str) -> Option<&'static str> {
    Some(match code {
        "auto" => "auto",
        "zh_cn" => "zh",
        "zh_tw" => "cht",
        "yue" => "yue",
        "en" => "en",
        "ja" => "jp",
        "ko" => "kor",
        "fr" => "fra",
        "es" => "spa",
        "ru" => "ru",
        "de" => "de",
        "it" => "it",
        "tr" => "tr",
        "pt_pt" => "pt",
        "pt_br" => "pot",
        "vi" => "vie",
        "id" => "id",
        "th" => "th",
        "ms" => "may",
        "ar" => "ar",
        "hi" => "hi",
        "km" => "hkm",
        "nb_no" => "nob",
        "nn_no" => "nno",
        "fa" => "per",
        "sv" => "swe",
        "pl" => "pl",
        "nl" => "nl",
        "uk" => "ukr",
        "he" => "heb",
        _ => return None,
    })
}

/// `from` / `to` 是 TiLex 语言码（`auto`、`zh_cn` …）。
pub fn translate(text: &str, from: &str, to: &str, config: &Config) -> Result<Value, Error> {
    let (Some(from), Some(to)) = (lang(from), lang(to)) else {
        return Err(Error::LanguageUnsupported);
    };
    if to == "auto" {
        return Err(Error::LanguageUnsupported);
    }
    let appid = config.appid.trim();
    if appid.is_empty() {
        return Err(Error::NotConfigured("appid"));
    }
    let secret = config.secret.trim();
    if secret.is_empty() {
        return Err(Error::NotConfigured("secret"));
    }

    let salt = generate_salt();
    let sign = signature(appid, text, &salt, secret);

    let query = [
        ("q", text),
        ("from", from),
        ("to", to),
        ("appid", appid),
        ("salt", salt.as_str()),
        ("sign", sign.as_str()),
    ];

    let data = http::get(URL, &query, &[], http::TIMEOUT_TRANSLATE)?;
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

static SALT_COUNTER: AtomicU64 = AtomicU64::new(0);

fn generate_salt() -> String {
    let count = SALT_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos}{count}")
}

/// 纯函数签名计算：方便固定输入做回归测试。
/// 规则：`md5(appid + text + salt + secret)`，十六进制小写。
pub(crate) fn signature(appid: &str, text: &str, salt: &str, secret: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(appid.as_bytes());
    hasher.update(text.as_bytes());
    hasher.update(salt.as_bytes());
    hasher.update(secret.as_bytes());
    let hash = hasher.finalize();

    const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut hex = String::with_capacity(32);
    for &b in &hash {
        hex.push(HEX_DIGITS[(b >> 4) as usize] as char);
        hex.push(HEX_DIGITS[(b & 0x0F) as usize] as char);
    }
    hex
}

pub(crate) fn parse_response(data: &Value) -> Option<String> {
    // 先看 trans_result 再看 error_code：百度成功响应里没有 error_code，但 52000 本身就是"成功"，
    // 万一哪天带上了，先判 error_code 会把好好的译文丢掉（旧版只看 trans_result）。
    let Some(trans_result) = data.get("trans_result").and_then(Value::as_array) else {
        if let Some(err_code) = data.get("error_code") {
            log::warn!("Baidu: error_code={err_code}");
        }
        return None;
    };
    if trans_result.is_empty() {
        return None;
    }
    let mut out = String::new();
    for item in trans_result {
        let dst = item.get("dst")?.as_str()?;
        out.push_str(dst);
        out.push('\n');
    }
    let trimmed = out.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn signature_matches_fixed_known_output() {
        let appid = "2015063000000001";
        let text = "apple";
        let salt = "1435660288";
        let secret = "12345678";
        let sig = signature(appid, text, salt, secret);
        assert_eq!(sig, "f89f9594663708c1605f3d736d01d2d4");
    }

    #[test]
    fn parse_response_extracts_translation() {
        let single = json!({
            "from": "en",
            "to": "zh",
            "trans_result": [
                {
                    "src": "Hello world",
                    "dst": "你好，世界"
                }
            ]
        });
        assert_eq!(parse_response(&single), Some("你好，世界".to_owned()));

        let multi = json!({
            "from": "en",
            "to": "zh",
            "trans_result": [
                { "src": "line 1", "dst": "第一行" },
                { "src": "line 2", "dst": "第二行" }
            ]
        });
        assert_eq!(parse_response(&multi), Some("第一行\n第二行".to_owned()));
    }

    #[test]
    fn parse_response_handles_error_code_and_invalid_shapes() {
        let error_code_str = json!({
            "error_code": "54001",
            "error_msg": "Invalid Sign"
        });
        assert_eq!(parse_response(&error_code_str), None);

        let error_code_num = json!({
            "error_code": 52003,
            "error_msg": "UNAUTHORIZED USER"
        });
        assert_eq!(parse_response(&error_code_num), None);

        let err = parse_response(&error_code_str)
            .map(Value::String)
            .ok_or_else(bad_response)
            .unwrap_err();
        assert_eq!(err.to_string(), "unexpected response from the server");

        let empty_result = json!({
            "trans_result": []
        });
        assert_eq!(parse_response(&empty_result), None);

        let empty_dst = json!({
            "trans_result": [{ "src": "a", "dst": "   " }]
        });
        assert_eq!(parse_response(&empty_dst), None);

        let missing_result = json!({});
        assert_eq!(parse_response(&missing_result), None);

        // 52000 是"成功"，带着译文一起回来时不能当失败丢掉
        let success_code_with_result = json!({
            "error_code": "52000",
            "trans_result": [{ "src": "apple", "dst": "苹果" }]
        });
        assert_eq!(
            parse_response(&success_code_with_result),
            Some("苹果".to_owned())
        );
    }

    #[test]
    fn language_table() {
        assert_eq!(lang("zh_cn"), Some("zh"));
        assert_eq!(lang("zh_tw"), Some("cht"));
        assert_eq!(lang("en"), Some("en"));
        assert_eq!(lang("auto"), Some("auto"));
        assert_eq!(lang("unsupported_lang"), None);

        let config = Config {
            appid: "id".into(),
            secret: "sec".into(),
        };
        assert!(matches!(
            translate("hello", "unsupported_lang", "zh_cn", &config),
            Err(Error::LanguageUnsupported)
        ));
        assert!(matches!(
            translate("hello", "en", "auto", &config),
            Err(Error::LanguageUnsupported)
        ));
    }

    #[test]
    fn missing_settings_fail_before_any_request() {
        let empty_appid = Config {
            appid: "   ".into(),
            secret: "sec".into(),
        };
        assert!(matches!(
            translate("hello", "auto", "zh_cn", &empty_appid),
            Err(Error::NotConfigured("appid"))
        ));

        let empty_secret = Config {
            appid: "id".into(),
            secret: "".into(),
        };
        assert!(matches!(
            translate("hello", "auto", "zh_cn", &empty_secret),
            Err(Error::NotConfigured("secret"))
        ));
    }

    #[test]
    fn salt_generation_changes_consecutively() {
        let s1 = generate_salt();
        let s2 = generate_salt();
        assert_ne!(s1, s2);
        assert!(!s1.is_empty());
        assert!(!s2.is_empty());
    }

    // 没有 live 测试：百度要 appid / secret，拿不到真 key 的断言只能写成永真，
    // 那比没有测试更糟。签名由上面的官方文档示例守着。
}
