//! 微软必应翻译，两种模式（从旧 `services/translate/bing/index.jsx` 移植）：
//! builtin（默认，免 key 内置接口，HMAC-SHA256 签名）、api（Azure 官方 API，要 key）。
//! 返回值：纯译文是 `Value::String`（`trim()` 过）。

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;

use crate::error::{Error, HttpKind};
use crate::service::http;

const API_DEFAULT: &str = "https://api.cognitive.microsofttranslator.com";

const PRIVATE_KEY_HEX: &str = "a2293a3dd0dd3273977a64dbc2f327f5d7bf87d9459df05a0966c630c66aaa849a41aa943aa8d51a6e4daac9a3701235c7eb12f6e823079e471095918855d817";

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// `builtin`（缺省）/ `api`；认不出的按 builtin。
    #[serde(skip_serializing_if = "String::is_empty", alias = "type")]
    pub mode: String,
    /// api 模式的 Azure Translator Key。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub auth_key: String,
    /// api 模式的区域（Region），留空为全局。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub region: String,
    /// api 模式的 endpoint。空 = 默认官方地址。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub custom_url: String,
}

/// TiLex 语言码 → 必应语言码。不在表里的就是这个服务不支持。
fn lang(code: &str) -> Option<&'static str> {
    Some(match code {
        "auto" => "",
        "zh_cn" => "zh-Hans",
        "zh_tw" => "zh-Hant",
        "yue" => "yue",
        "en" => "en",
        "ja" => "ja",
        "ko" => "ko",
        "fr" => "fr",
        "es" => "es",
        "ru" => "ru",
        "de" => "de",
        "it" => "it",
        "tr" => "tr",
        "pt_pt" => "pt-pt",
        "pt_br" => "pt",
        "vi" => "vi",
        "id" => "id",
        "th" => "th",
        "ms" => "ms",
        "ar" => "ar",
        "hi" => "hi",
        "mn_cy" => "mn-Cyrl",
        "mn_mo" => "mn-Mong",
        "km" => "km",
        "nb_no" => "nb",
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
    if to.is_empty() {
        return Err(Error::LanguageUnsupported);
    }
    match config.mode.as_str() {
        "api" => by_api(text, from, to, config),
        _ => by_builtin(text, from, to),
    }
}

fn base(url: &str, default: &str) -> String {
    let url = url.trim();
    let url = if url.is_empty() { default } else { url };
    let url = if url.starts_with("http://") || url.starts_with("https://") {
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

const fn hex_val(b: u8) -> u8 {
    match b {
        b'0'..=b'9' => b - b'0',
        b'a'..=b'f' => b - b'a' + 10,
        b'A'..=b'F' => b - b'A' + 10,
        _ => 0,
    }
}

fn private_key() -> &'static [u8; 64] {
    static KEY: OnceLock<[u8; 64]> = OnceLock::new();
    KEY.get_or_init(|| {
        let mut bytes = [0u8; 64];
        for (i, chunk) in PRIVATE_KEY_HEX
            .as_bytes()
            .as_chunks::<2>()
            .0
            .iter()
            .enumerate()
        {
            let hi = hex_val(chunk[0]);
            let lo = hex_val(chunk[1]);
            bytes[i] = (hi << 4) | lo;
        }
        bytes
    })
}

/// JS encodeURIComponent 语义：保留 A-Za-z0-9 - _ . ! ~ * ' ( )，其余逐字节 %XX 大写编码。
pub(crate) fn encode_uri_component(s: &str) -> String {
    const HEX_DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut out = String::with_capacity(s.len() * 3 / 2);
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(HEX_DIGITS[(b >> 4) as usize] as char);
                out.push(HEX_DIGITS[(b & 0x0F) as usize] as char);
            }
        }
    }
    out
}

/// 纯函数签名计算：方便固定输入做回归测试。
pub(crate) fn signature(request_path: &str, date_time: &str, guid: &str) -> String {
    let escaped_url = encode_uri_component(request_path);
    let sign_str = format!("MSTranslatorAndroidApp{escaped_url}{date_time}{guid}").to_lowercase();
    #[allow(clippy::expect_used, reason = "HMAC accepts any key length")]
    let mut mac = HmacSha256::new_from_slice(private_key()).expect("HMAC accepts any key length");
    mac.update(sign_str.as_bytes());
    let hash_bytes = mac.finalize().into_bytes();
    let hash = BASE64_STANDARD.encode(hash_bytes);
    format!("MSTranslatorAndroidApp::{hash}::{date_time}::{guid}")
}

static GUID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn random_guid() -> String {
    let count = GUID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let hi =
        (nanos ^ count.wrapping_mul(0x9e37_79b9_7f4a_7c15)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    let lo = (nanos.rotate_left(32) ^ count).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    format!("{hi:016x}{lo:016x}")
}

fn by_builtin(text: &str, from: &str, to: &str) -> Result<Value, Error> {
    let mut request_path =
        format!("api.cognitive.microsofttranslator.com/translate?api-version=3.0&to={to}");
    if !from.is_empty() && from != "auto" {
        request_path.push_str(&format!("&from={from}"));
    }

    let date_time = chrono::Utc::now()
        .format("%a, %d %b %Y %H:%M:%S GMT")
        .to_string();
    let guid = random_guid();
    let sig = signature(&request_path, &date_time, &guid);

    let url = format!("https://{request_path}");
    let headers = [
        (
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/127.0.0.0 Safari/537.36",
        ),
        ("X-MT-Signature", sig.as_str()),
    ];
    let body = json!([{ "Text": text }]);
    let data = http::post_json(&url, &headers, &body, http::TIMEOUT_TRANSLATE)?;
    parse_response(&data)
        .map(Value::String)
        .ok_or_else(bad_response)
}

fn by_api(text: &str, from: &str, to: &str, config: &Config) -> Result<Value, Error> {
    let key = config.auth_key.trim();
    if key.is_empty() {
        return Err(Error::NotConfigured("auth_key"));
    }
    let endpoint = base(&config.custom_url, API_DEFAULT);
    let mut url = format!("{endpoint}/translate?api-version=3.0&to={to}");
    if !from.is_empty() && from != "auto" {
        url.push_str(&format!("&from={from}"));
    }

    let mut headers = vec![("Ocp-Apim-Subscription-Key", key)];
    let region = config.region.trim();
    if !region.is_empty() {
        headers.push(("Ocp-Apim-Subscription-Region", region));
    }

    let body = json!([{ "Text": text }]);
    let data = http::post_json(&url, &headers, &body, http::TIMEOUT_TRANSLATE)?;
    parse_response(&data)
        .map(Value::String)
        .ok_or_else(bad_response)
}

fn parse_response(data: &Value) -> Option<String> {
    let text = data
        .get(0)?
        .get("translations")?
        .get(0)?
        .get("text")?
        .as_str()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_uri_component_escapes_expected_characters() {
        let raw = "api.cognitive.microsofttranslator.com/translate?api-version=3.0&to=zh-Hans";
        let encoded = encode_uri_component(raw);
        assert_eq!(
            encoded,
            "api.cognitive.microsofttranslator.com%2Ftranslate%3Fapi-version%3D3.0%26to%3Dzh-Hans"
        );
        assert_eq!(
            encode_uri_component("a b c ~ ! * ( ) ' - _ ."),
            "a%20b%20c%20~%20!%20*%20(%20)%20'%20-%20_%20."
        );
    }

    #[test]
    fn signature_matches_fixed_known_output() {
        let request_path =
            "api.cognitive.microsofttranslator.com/translate?api-version=3.0&to=zh-Hans";
        let date_time = "Thu, 18 Sep 2026 07:00:00 GMT";
        let guid = "0123456789abcdef0123456789abcdef";
        let sig = signature(request_path, date_time, guid);

        // 待签串转小写后 HMAC-SHA256
        let escaped = encode_uri_component(request_path);
        let sign_str = format!("MSTranslatorAndroidApp{escaped}{date_time}{guid}").to_lowercase();
        let mut mac = HmacSha256::new_from_slice(private_key()).unwrap();
        mac.update(sign_str.as_bytes());
        let expected_hash = BASE64_STANDARD.encode(mac.finalize().into_bytes());

        let expected_sig = format!("MSTranslatorAndroidApp::{expected_hash}::{date_time}::{guid}");
        assert_eq!(sig, expected_sig);
        assert!(sig.starts_with("MSTranslatorAndroidApp::"));
        assert!(sig.ends_with("::0123456789abcdef0123456789abcdef"));
    }

    #[test]
    fn parse_response_extracts_translation_or_fails() {
        let valid = json!([
            {
                "detectedLanguage": {"language": "en", "score": 1.0},
                "translations": [
                    {
                        "text": "  你好世界  ",
                        "to": "zh-Hans"
                    }
                ]
            }
        ]);
        assert_eq!(parse_response(&valid), Some("你好世界".to_owned()));

        let empty_text = json!([{"translations": [{"text": "   "}]}]);
        assert_eq!(parse_response(&empty_text), None);

        let error_shape = json!({"error": {"code": 401000, "message": "Invalid key"}});
        assert_eq!(parse_response(&error_shape), None);

        let empty_arr = json!([]);
        assert_eq!(parse_response(&empty_arr), None);
    }

    #[test]
    fn language_table() {
        assert_eq!(lang("zh_cn"), Some("zh-Hans"));
        assert_eq!(lang("zh_tw"), Some("zh-Hant"));
        assert_eq!(lang("auto"), Some(""));
        assert_eq!(lang("unsupported_lang"), None);
        assert!(matches!(
            translate("hello", "unsupported_lang", "zh_cn", &Config::default()),
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
            auth_key: "   ".into(),
            ..Config::default()
        };
        assert!(matches!(
            translate("hello", "auto", "zh_cn", &api_config),
            Err(Error::NotConfigured("auth_key"))
        ));
    }

    #[test]
    fn base_url_formatting() {
        assert_eq!(base("", API_DEFAULT), API_DEFAULT);
        assert_eq!(
            base(" api.translator.azure.cn/ ", API_DEFAULT),
            "https://api.translator.azure.cn"
        );
        assert_eq!(
            base("http://custom.endpoint/", API_DEFAULT),
            "http://custom.endpoint"
        );
    }

    #[test]
    fn random_guid_changes_both_halves_consecutively() {
        let g1 = random_guid();
        let g2 = random_guid();
        assert_eq!(g1.len(), 32);
        assert_eq!(g2.len(), 32);
        assert!(g1.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(g2.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(g1, g2);
        assert_ne!(&g1[..16], &g2[..16], "前 16 位两次不能相等");
    }

    #[test]
    #[ignore]
    fn live_builtin_translation() {
        let out = translate("Hello world", "auto", "zh_cn", &Config::default()).unwrap();
        assert!(out.as_str().is_some_and(|s| !s.is_empty()), "{out}");
    }
}
