//! DeepL 翻译，三种模式（从旧 `services/translate/deepl/index.jsx` 移植）：
//! free（默认，免 key 网页接口，时间戳对齐与指纹替换）、api（DeepL 官方 API，按后缀选端点）、
//! deeplx（自建 DeepLX 中转）。
//! 返回值：纯译文是 `Value::String`（`trim()` 过）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Error, HttpKind};
use crate::service::http;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `free`（缺省）/ `api` / `deeplx`；认不出的按 free。
    #[serde(skip_serializing_if = "String::is_empty", alias = "type")]
    pub mode: String,
    /// api 模式的 DeepL Authentication Key。
    #[serde(skip_serializing_if = "String::is_empty", alias = "authKey")]
    pub auth_key: String,
    /// deeplx 模式的自定义中转地址。
    #[serde(skip_serializing_if = "String::is_empty", alias = "customUrl")]
    pub custom_url: String,
}

/// TiLex 语言码 → DeepL 语言码。不在表里的就是这个服务不支持。
fn lang(code: &str) -> Option<&'static str> {
    Some(match code {
        "auto" => "auto",
        "zh_cn" | "zh_tw" => "ZH",
        "ja" => "JA",
        "en" => "EN",
        "ko" => "KO",
        "fr" => "FR",
        "es" => "ES",
        "ru" => "RU",
        "de" => "DE",
        "it" => "IT",
        "tr" => "TR",
        "pt_pt" => "PT-PT",
        "pt_br" => "PT-BR",
        "id" => "ID",
        "sv" => "SV",
        "pl" => "PL",
        "nl" => "NL",
        "uk" => "UK",
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
    match config.mode.as_str() {
        "api" => by_api(text, from, to, config),
        "deeplx" => by_deeplx(text, from, to, &config.custom_url),
        _ => by_free(text, from, to),
    }
}

fn bad_response() -> Error {
    Error::Http {
        status: Some(200),
        kind: HttpKind::Format,
    }
}

static RAND_COUNTER: AtomicU64 = AtomicU64::new(0);

fn random_id() -> u64 {
    let count = RAND_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let rand = ((nanos ^ count) % 99_999) + 100_000;
    rand * 1_000
}

/// 按文本中 'i' 的个数对齐毫秒时间戳。
pub(crate) fn timestamp(i_count: u64, ts: u64) -> u64 {
    if i_count != 0 {
        let count = i_count + 1;
        ts - (ts % count) + count
    } else {
        ts
    }
}

/// DeepL free 模式的服务端指纹：根据随机数替换 `"method":"` 里的空格。
pub(crate) fn format_fingerprint(body_str: &str, rand: u64) -> String {
    let replacement = if (rand + 5).is_multiple_of(29) || (rand + 3).is_multiple_of(13) {
        "\"method\" : \""
    } else {
        "\"method\": \""
    };
    body_str.replace("\"method\":\"", replacement)
}

#[derive(Serialize)]
struct FreeRequestBody<'a> {
    jsonrpc: &'a str,
    method: &'a str,
    params: FreeParams<'a>,
    id: u64,
}

#[derive(Serialize)]
struct FreeParams<'a> {
    splitting: &'a str,
    lang: FreeLang<'a>,
    texts: Vec<FreeText<'a>>,
    timestamp: u64,
}

#[derive(Serialize)]
struct FreeLang<'a> {
    source_lang_user_selected: &'a str,
    target_lang: &'a str,
}

#[derive(Serialize)]
struct FreeText<'a> {
    text: &'a str,
    #[serde(rename = "requestAlternatives")]
    request_alternatives: u32,
}

pub(crate) fn serialize_free_body(
    text: &str,
    from_sliced: &str,
    to_sliced: &str,
    timestamp: u64,
    id: u64,
) -> String {
    let body = FreeRequestBody {
        jsonrpc: "2.0",
        method: "LMT_handle_texts",
        params: FreeParams {
            splitting: "newlines",
            lang: FreeLang {
                source_lang_user_selected: from_sliced,
                target_lang: to_sliced,
            },
            texts: vec![FreeText {
                text,
                request_alternatives: 3,
            }],
            timestamp,
        },
        id,
    };
    #[allow(
        clippy::expect_used,
        reason = "全是 &str / u64 / Vec，serde_json 没有能失败的路径；发空 body 只会换来一次莫名的拒绝"
    )]
    serde_json::to_string(&body).expect("free request body is always serializable")
}

fn by_free(text: &str, from: &str, to: &str) -> Result<Value, Error> {
    const URL: &str = "https://www2.deepl.com/jsonrpc";
    let rand = random_id();
    let from_sliced = if from != "auto" {
        &from[..from.len().min(2)]
    } else {
        "auto"
    };
    let to_sliced = &to[..to.len().min(2)];
    let i_count = text.chars().filter(|c| *c == 'i').count() as u64;
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let ts_val = timestamp(i_count, ts);

    let body_str = serialize_free_body(text, from_sliced, to_sliced, ts_val, rand);
    let body_str = format_fingerprint(&body_str, rand);
    let data = http::post_text(URL, &[], &body_str, http::TIMEOUT_TRANSLATE)?;
    parse_free(&data)
        .map(Value::String)
        .ok_or_else(bad_response)
}

fn by_api(text: &str, from: &str, to: &str, config: &Config) -> Result<Value, Error> {
    let key = config.auth_key.trim();
    if key.is_empty() {
        return Err(Error::NotConfigured("auth_key"));
    }
    let url = if key.ends_with(":fx") {
        "https://api-free.deepl.com/v2/translate"
    } else if key.ends_with(":dp") {
        "https://api.deepl-pro.com/v2/translate"
    } else {
        "https://api.deepl.com/v2/translate"
    };

    let auth = format!("DeepL-Auth-Key {key}");
    let headers = [("Authorization", auth.as_str())];
    let mut body = json!({
        "text": [text],
        "target_lang": to,
    });
    if from != "auto" {
        body["source_lang"] = from.into();
    }
    let data = http::post_json(url, &headers, &body, http::TIMEOUT_TRANSLATE)?;
    parse_api(&data).map(Value::String).ok_or_else(bad_response)
}

fn by_deeplx(text: &str, from: &str, to: &str, custom_url: &str) -> Result<Value, Error> {
    let url = custom_url.trim();
    if url.is_empty() {
        return Err(Error::NotConfigured("custom_url"));
    }
    let full_url = if url.starts_with("http://") || url.starts_with("https://") {
        url.to_owned()
    } else {
        format!("https://{url}")
    };

    let body = json!({
        "source_lang": from,
        "target_lang": to,
        "text": text,
    });
    let data = http::post_json(&full_url, &[], &body, http::TIMEOUT_TRANSLATE)?;
    parse_deeplx(&data)
        .map(Value::String)
        .ok_or_else(bad_response)
}

fn parse_free(data: &Value) -> Option<String> {
    let text = data
        .pointer("/result/texts/0/text")
        .and_then(Value::as_str)?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn parse_api(data: &Value) -> Option<String> {
    let text = data
        .pointer("/translations/0/text")
        .and_then(Value::as_str)?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn parse_deeplx(data: &Value) -> Option<String> {
    let text = data.get("data").and_then(Value::as_str)?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_aligns_with_i_count() {
        // i_count == 0 时原样返回
        assert_eq!(timestamp(0, 1600000005), 1600000005);

        // 期望值写成字面量：照公式再算一遍等于把实现抄进测试，算错了也发现不了
        assert_eq!(timestamp(2, 100), 102); // count=3: 100 - 1 + 3
        assert_eq!(timestamp(1, 100), 102); // count=2: 100 - 0 + 2
        assert_eq!(timestamp(4, 100), 105); // count=5: 100 - 0 + 5
    }

    #[test]
    fn format_fingerprint_replaces_method_tag() {
        let raw = "{\"jsonrpc\":\"2.0\",\"method\":\"LMT_handle_texts\"}";

        // (rand + 5) % 29 == 0 时带空格
        let rand_with_space = 24; // (24 + 5) % 29 == 0
        let replaced_space = format_fingerprint(raw, rand_with_space);
        assert_eq!(
            replaced_space,
            "{\"jsonrpc\":\"2.0\",\"method\" : \"LMT_handle_texts\"}"
        );

        // 普通情况
        let rand_normal = 1; // (1 + 5) % 29 == 6, (1 + 3) % 13 == 4
        let replaced_normal = format_fingerprint(raw, rand_normal);
        assert_eq!(
            replaced_normal,
            "{\"jsonrpc\":\"2.0\",\"method\": \"LMT_handle_texts\"}"
        );
    }

    #[test]
    fn parse_free_response() {
        let valid = json!({
            "jsonrpc": "2.0",
            "result": {
                "texts": [
                    { "text": "  你好世界  " }
                ]
            }
        });
        assert_eq!(parse_free(&valid), Some("你好世界".to_owned()));

        let empty = json!({"jsonrpc": "2.0", "result": {"texts": [{"text": " "}]}});
        assert_eq!(parse_free(&empty), None);

        let error = json!({"jsonrpc": "2.0", "error": {"code": -32600, "message": "Invalid"}});
        assert_eq!(parse_free(&error), None);
    }

    #[test]
    fn parse_api_response() {
        let valid = json!({
            "translations": [
                {
                    "detected_source_language": "EN",
                    "text": "  Hallo Welt  "
                }
            ]
        });
        assert_eq!(parse_api(&valid), Some("Hallo Welt".to_owned()));

        let empty = json!({"translations": [{"text": ""}]});
        assert_eq!(parse_api(&empty), None);

        let missing = json!({"other": 1});
        assert_eq!(parse_api(&missing), None);
    }

    #[test]
    fn parse_deeplx_response() {
        let valid = json!({
            "code": 200,
            "data": "  译文内容  "
        });
        assert_eq!(parse_deeplx(&valid), Some("译文内容".to_owned()));

        let empty = json!({"code": 200, "data": "  "});
        assert_eq!(parse_deeplx(&empty), None);

        let missing = json!({"code": 500});
        assert_eq!(parse_deeplx(&missing), None);
    }

    #[test]
    fn language_table() {
        assert_eq!(lang("zh_cn"), Some("ZH"));
        assert_eq!(lang("zh_tw"), Some("ZH"));
        assert_eq!(lang("pt_pt"), Some("PT-PT"));
        assert_eq!(lang("auto"), Some("auto"));
        assert_eq!(lang("unsupported"), None);

        assert!(matches!(
            translate("hello", "unsupported", "zh_cn", &Config::default()),
            Err(Error::LanguageUnsupported)
        ));
        assert!(matches!(
            translate("hello", "en", "auto", &Config::default()),
            Err(Error::LanguageUnsupported)
        ));
    }

    #[test]
    fn missing_settings_fail_before_any_request() {
        let api_config = Config {
            mode: "api".into(),
            auth_key: "".into(),
            ..Config::default()
        };
        assert!(matches!(
            translate("hello", "auto", "zh_cn", &api_config),
            Err(Error::NotConfigured("auth_key"))
        ));

        let deeplx_config = Config {
            mode: "deeplx".into(),
            custom_url: "   ".into(),
            ..Config::default()
        };
        assert!(matches!(
            translate("hello", "auto", "zh_cn", &deeplx_config),
            Err(Error::NotConfigured("custom_url"))
        ));
    }

    #[test]
    fn free_body_field_order_matches_legacy_shape() {
        let s = serialize_free_body("hello", "en", "zh", 12345, 99999);
        assert!(s.starts_with("{\"jsonrpc\":\"2.0\",\"method\":"), "{s}");
        let params_pos = s.find("\"params\":").expect("must contain params");
        let id_pos = s.find("\"id\":").expect("must contain id");
        assert!(id_pos > params_pos, "id must appear after params: {s}");
    }

    #[test]
    #[ignore]
    fn live_free_translation() {
        let out = translate("Hello world", "auto", "zh_cn", &Config::default()).unwrap();
        assert!(out.as_str().is_some_and(|s| !s.is_empty()), "{out}");
    }
}
