//! 谷歌翻译，三种模式（从旧 `services/translate/google/index.jsx` 移植）：
//! web（默认，免 key，可换镜像）、api（Google Cloud v2，要 key）、custom_api（用户自建中转）。
//! 返回值：纯译文是 `Value::String`；单词命中词典时是旧版词典形状的对象（没有 `kind`），由业务层规范化。

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Error, HttpKind};
use crate::service::http;

const WEB_DEFAULT: &str = "https://translate.google.com";
const API_DEFAULT: &str = "https://translation.googleapis.com";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `web`（缺省）/ `api` / `custom_api`；认不出的按 web。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub mode: String,
    /// web：镜像地址；api：endpoint。空 = 官方地址。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub custom_url: String,
    /// api 模式的 Google Cloud key。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub api_key: String,
    /// custom_api 模式的完整地址。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub custom_api_url: String,
}

/// TiLex 语言码 → 谷歌语言码。不在表里的就是这个服务不支持。
fn lang(code: &str) -> Option<&'static str> {
    Some(match code {
        "auto" => "auto",
        "zh_cn" => "zh-CN",
        "zh_tw" => "zh-TW",
        "ja" => "ja",
        "en" => "en",
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
        "hi" => "hi",
        "mn_cy" => "mn",
        "km" => "km",
        "nb_no" | "nn_no" => "no",
        "fa" => "fa",
        "sv" => "sv",
        "pl" => "pl",
        "nl" => "nl",
        "uk" => "uk",
        "he" => "he",
        _ => return None,
    })
}

/// `from` / `to` 是 TiLex 语言码（`auto`、`zh_cn` …）。
pub fn translate(text: &str, from: &str, to: &str, config: &Config) -> Result<Value, Error> {
    let (Some(from), Some(to)) = (lang(from), lang(to)) else {
        return Err(Error::LanguageUnsupported);
    };
    match config.mode.as_str() {
        "api" => by_api(text, from, to, config),
        "custom_api" => by_custom_api(text, from, to, &config.custom_api_url),
        _ => by_web(text, from, to, &config.custom_url),
    }
}

fn base(url: &str, default: &str) -> String {
    let url = url.trim();
    let url = if url.is_empty() { default } else { url };
    let url = if url.starts_with("http") {
        url.to_owned()
    } else {
        format!("https://{url}")
    };
    url.trim_end_matches('/').to_owned()
}

fn bad_response() -> Error {
    Error::Http {
        status: Some(200),
        kind: HttpKind::Format,
    }
}

fn by_web(text: &str, from: &str, to: &str, custom_url: &str) -> Result<Value, Error> {
    let url = format!("{}/translate_a/single", base(custom_url, WEB_DEFAULT));
    let mut query: Vec<(&str, &str)> = ["at", "bd", "ex", "ld", "md", "qca", "rw", "rm", "ss", "t"]
        .iter()
        .map(|dt| ("dt", *dt))
        .collect();
    query.extend([
        ("client", "gtx"),
        ("sl", from),
        ("tl", to),
        ("hl", to),
        ("ie", "UTF-8"),
        ("oe", "UTF-8"),
        ("otf", "1"),
        ("ssel", "0"),
        ("tsel", "0"),
        ("kc", "7"),
        ("q", text),
    ]);
    let data = http::get(&url, &query, &[], http::TIMEOUT_TRANSLATE)?;
    parse_web(&data).ok_or_else(bad_response)
}

/// `data[1]` 有释义就是词典模式，否则拼 `data[0][i][0]` 成整句译文。空结果算格式错误（界面不出空白行）。
fn parse_web(data: &Value) -> Option<Value> {
    if let Some(dict) = dictionary(data) {
        return Some(dict);
    }
    let text: String = data
        .get(0)?
        .as_array()?
        .iter()
        .filter_map(|part| part.get(0).and_then(Value::as_str))
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| Value::String(text.to_owned()))
}

fn dictionary(data: &Value) -> Option<Value> {
    let explanations: Vec<Value> = data
        .get(1)?
        .as_array()?
        .iter()
        .filter_map(|entry| {
            let explains: Vec<&str> = entry
                .get(2)?
                .as_array()?
                .iter()
                .filter_map(|x| x.get(0).and_then(Value::as_str))
                .collect();
            let mut item = json!({ "explains": explains });
            if let Some(t) = entry.get(0).and_then(Value::as_str) {
                item["trait"] = t.into();
            }
            (!explains.is_empty()).then_some(item)
        })
        .collect();
    if explanations.is_empty() {
        return None;
    }
    let pronunciations: Vec<Value> = data
        .pointer("/0/1/3")
        .and_then(Value::as_str)
        .map(|symbol| json!({ "symbol": symbol }))
        .into_iter()
        .collect();
    Some(json!({
        "pronunciations": pronunciations,
        "explanations": explanations,
        "associations": [],
    }))
}

fn by_api(text: &str, from: &str, to: &str, config: &Config) -> Result<Value, Error> {
    let key = config.api_key.trim();
    if key.is_empty() {
        return Err(Error::NotConfigured("api_key"));
    }
    let url = format!(
        "{}/language/translate/v2",
        base(&config.custom_url, API_DEFAULT)
    );
    let mut body = json!({ "q": [text], "target": to, "format": "text" });
    if from != "auto" {
        body["source"] = from.into();
    }
    // key 走请求头不走地址：地址会出现在各种错误和代理日志里。
    let data = http::post_json(
        &url,
        &[("x-goog-api-key", key)],
        &body,
        http::TIMEOUT_TRANSLATE,
    )?;
    let translated = data
        .pointer("/data/translations/0/translatedText")
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty())
        .ok_or_else(bad_response)?;
    Ok(Value::String(decode_entities(translated).trim().to_owned()))
}

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&amp;", "&")
}

fn by_custom_api(text: &str, from: &str, to: &str, url: &str) -> Result<Value, Error> {
    if url.trim().is_empty() {
        return Err(Error::NotConfigured("custom_api_url"));
    }
    let url = base(url, "");
    let body = json!({ "text": text, "source_lang": from, "target_lang": to });
    let data = http::post_json(&url, &[], &body, http::TIMEOUT_TRANSLATE)?;
    parse_custom(&data).ok_or_else(bad_response)
}

/// 中转接口的几种常见形状：纯文本、`data`、`translation`、`result`。
fn parse_custom(data: &Value) -> Option<Value> {
    let text = match data {
        Value::String(s) => s.clone(),
        _ => ["data", "translation", "result"]
            .iter()
            .find_map(|k| match data.get(*k)? {
                Value::Null => None,
                Value::String(s) => Some(s.clone()),
                other => Some(other.to_string()),
            })?,
    };
    let text = text.trim();
    (!text.is_empty()).then(|| Value::String(text.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_sentence_joins_segments() {
        let data = json!([
            [["你好，", "Hello, ", null], ["世界 ", "world", null]],
            null,
            "en"
        ]);
        assert_eq!(parse_web(&data), Some(json!("你好，世界")));
        assert_eq!(
            parse_web(&json!([[], null, "en"])),
            None,
            "空结果不能成空白行"
        );
        assert_eq!(parse_web(&json!({"error": 1})), None);
    }

    #[test]
    fn web_dictionary_has_the_legacy_dictionary_shape() {
        let data = json!([
            [
                ["翻译", "translation", null, null],
                [null, null, null, "trænsˈleɪʃən"]
            ],
            [[
                "名词",
                ["翻译", "译文"],
                [["翻译", ["translation"]], ["译文", ["version"]], [3]]
            ]],
        ]);
        assert_eq!(
            parse_web(&data),
            Some(json!({
                "pronunciations": [{"symbol": "trænsˈleɪʃən"}],
                "explanations": [{"trait": "名词", "explains": ["翻译", "译文"]}],
                "associations": [],
            }))
        );
        // data[1] 是空数组：退回整句译文
        assert_eq!(parse_web(&json!([[["词", "w"]], []])), Some(json!("词")));
    }

    #[test]
    fn language_table() {
        assert_eq!(lang("pt_br"), Some("pt"));
        assert_eq!(lang("mn_mo"), None);
        assert!(matches!(
            translate("x", "mn_mo", "en", &Config::default()),
            Err(Error::LanguageUnsupported)
        ));
    }

    #[test]
    fn missing_settings_fail_before_any_request() {
        let api = Config {
            mode: "api".into(),
            ..Config::default()
        };
        assert!(matches!(
            translate("x", "auto", "zh_cn", &api),
            Err(Error::NotConfigured("api_key"))
        ));
        let custom = Config {
            mode: "custom_api".into(),
            ..Config::default()
        };
        assert!(matches!(
            translate("x", "auto", "zh_cn", &custom),
            Err(Error::NotConfigured("custom_api_url"))
        ));
    }

    #[test]
    fn helpers() {
        assert_eq!(base("", WEB_DEFAULT), WEB_DEFAULT);
        assert_eq!(
            base(" mirror.example/ ", WEB_DEFAULT),
            "https://mirror.example"
        );
        assert_eq!(decode_entities("a &amp;lt; b &#39;c&#39;"), "a &lt; b 'c'");
        assert_eq!(parse_custom(&json!(" 译文 ")), Some(json!("译文")));
        assert_eq!(parse_custom(&json!({"data": "d"})), Some(json!("d")));
        assert_eq!(parse_custom(&json!({"translation": "t"})), Some(json!("t")));
        assert_eq!(
            parse_custom(&json!({"result": {"a": 1}})),
            Some(json!("{\"a\":1}"))
        );
        assert_eq!(parse_custom(&json!({"other": 1})), None);
    }

    // 真联网（走系统代理）。cargo test --release --features detect-full google -- --ignored
    #[test]
    #[ignore]
    fn live_web_translation() {
        let out = translate("Hello world", "auto", "zh_cn", &Config::default()).unwrap();
        assert!(out.as_str().is_some_and(|s| !s.is_empty()), "{out}");
        let word = translate("translation", "en", "zh_cn", &Config::default()).unwrap();
        assert!(word.get("explanations").is_some(), "{word}");
    }
}
