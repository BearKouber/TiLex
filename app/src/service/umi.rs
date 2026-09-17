//! Umi-OCR 本地离线识别服务（从旧 `services/recognize/umi/` 移植）。
//!
//! 请求：`POST <url>`，body `{"base64": "<图片字节的 base64，不带 data: 前缀>"}`。
//! 超时 30 秒（design §2.5）。发请求走 `crate::service::http`（R-4）。

use std::path::Path;
use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{Error, HttpKind};
use crate::service::http;

pub const DEFAULT_URL: &str = "http://127.0.0.1:1224/api/ocr";
const TIMEOUT_UMI: Duration = Duration::from_secs(30);

/// 1×1 的白点 PNG。测试连接只是拿它敲一下端口：认不出字（code 101）也算连上了
/// （旧版 `Recognize/ConfigModal/index.jsx` 的 `PING_PNG`）。
const PING_PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// Umi-OCR 本地离线识别。免费、不联网，代价是用户得自己开着 Umi-OCR
/// 并把 HTTP 接口打开（设置 → 全局设置 → 剪贴板/HTTP服务，默认端口 1224）。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub url: String, // 默认 "http://127.0.0.1:1224/api/ocr"
}

impl Default for Config {
    fn default() -> Self {
        Self {
            url: DEFAULT_URL.to_owned(),
        }
    }
}

fn effective_url(url: &str) -> &str {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        DEFAULT_URL
    } else {
        trimmed
    }
}

/// 识别一张图。几百毫秒到几秒，**不要在 UI 线程上调**（R-5）。
///
/// 连不上（Umi 没启动）时底层返回 `Error::Http { kind: HttpKind::Connect, .. }`，
/// 原样往上抛，不在此处拼中文文案。文案由前端在 `.slint` 的 `Strings` 中根据
/// 「服务是 umi + 错误是 Connect」显示（design §2.9）。
pub fn recognize(config: &Config, image: &Path) -> Result<String, Error> {
    let bytes = std::fs::read(image)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    post_base64(config, &encoded)
}

/// 测试连接：发一张 1×1 的白点过去，能回话就算通（识别不出字也算）。
/// 连不上时同样原样抛 `Error::Http { kind: Connect }`，文案由界面层给。
pub fn test(config: &Config) -> Result<(), Error> {
    post_base64(config, PING_PNG).map(|_| ())
}

fn post_base64(config: &Config, encoded: &str) -> Result<String, Error> {
    let url = effective_url(&config.url);
    let body = serde_json::json!({ "base64": encoded });
    let resp = http::post_json(url, &[], &body, TIMEOUT_UMI)?;
    parse(&resp)
}

fn bad_response() -> Error {
    Error::Http {
        status: Some(200),
        kind: HttpKind::Format,
    }
}

/// 响应判读纯函数。
///
/// - `code == 100`：提取 `data` 数组中每个块的 `text`，丢弃空白，换行拼接。
///   保留原有顺序，**不进行排序**（Umi-OCR 已按阅读顺序返回）。
/// - `code == 101`：图里无文字，返回空串 `Ok("")`，不作为错误处理。
/// - 其他 code：若 `data` 为字符串则作为错误消息，否则为 `Umi-OCR: code=<code>`。
///   当前权宜返回 `Error::Platform(msg)`，待第 6 轮接界面时如需细分再议。
fn parse(json: &Value) -> Result<String, Error> {
    let Some(code) = json.get("code").and_then(Value::as_i64) else {
        return Err(bad_response());
    };
    if code == 100 {
        let Some(data) = json.get("data").and_then(Value::as_array) else {
            return Err(bad_response());
        };
        let lines: Vec<&str> = data
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .filter(|t| !t.trim().is_empty())
            .collect();
        return Ok(lines.join("\n"));
    }
    if code == 101 {
        return Ok(String::new());
    }
    let msg = match json.get("data").and_then(Value::as_str) {
        Some(s) => s.to_owned(),
        None => format!("Umi-OCR: code={code}"),
    };
    // 权宜：Error 枚举本轮不许加变体，这里将 Umi 错误文案包成 Error::Platform 返回；第 6 轮接界面时再议。
    Err(Error::Platform(msg))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_code_100_preserves_order_and_filters_empty() {
        // 造一个 top 递减的数组，断言输出顺序和输入一致，绝不能被重新排序
        let input = json!({
            "code": 100,
            "data": [
                {"text": "first line", "box": [0, 500, 100, 520]},
                {"text": "   ", "box": [0, 400, 100, 420]},
                {"text": "second line", "box": [0, 300, 100, 320]},
                {"text": "", "box": [0, 200, 100, 220]},
                {"text": "third line", "box": [0, 100, 100, 120]},
            ]
        });
        assert_eq!(
            parse(&input).unwrap(),
            "first line\nsecond line\nthird line"
        );
    }

    #[test]
    fn parse_code_101_returns_empty_string() {
        let input = json!({
            "code": 101,
            "data": "No text found in image"
        });
        assert_eq!(parse(&input).unwrap(), "");
    }

    #[test]
    fn parse_other_codes_returns_platform_error() {
        let err_str = json!({
            "code": 102,
            "data": "图片格式错误"
        });
        match parse(&err_str) {
            Err(Error::Platform(msg)) => assert_eq!(msg, "图片格式错误"),
            other => panic!("expected Error::Platform, got {other:?}"),
        }

        let err_no_str = json!({
            "code": 999
        });
        match parse(&err_no_str) {
            Err(Error::Platform(msg)) => assert!(msg.contains("code=999"), "{msg}"),
            other => panic!("expected Error::Platform, got {other:?}"),
        }

        let err_obj_data = json!({
            "code": 500,
            "data": {"detail": "internal error"}
        });
        match parse(&err_obj_data) {
            Err(Error::Platform(msg)) => assert!(msg.contains("code=500"), "{msg}"),
            other => panic!("expected Error::Platform, got {other:?}"),
        }
    }

    #[test]
    fn parse_bad_shape_returns_error_without_panic() {
        // code 缺失
        assert!(parse(&json!({"data": []})).is_err());
        // code 不是数字
        assert!(parse(&json!({"code": "100", "data": []})).is_err());
        assert!(parse(&json!({"code": true, "data": []})).is_err());
        // code == 100 但 data 不是数组
        assert!(parse(&json!({"code": 100, "data": "not array"})).is_err());
        assert!(parse(&json!({"code": 100, "data": 123})).is_err());
        assert!(parse(&json!({"code": 100})).is_err());
    }

    #[test]
    fn config_url_fallback() {
        let default_config = Config::default();
        assert_eq!(default_config.url, DEFAULT_URL);
        assert_eq!(effective_url(&default_config.url), DEFAULT_URL);
        assert_eq!(effective_url(""), DEFAULT_URL);
        assert_eq!(effective_url("   "), DEFAULT_URL);
        assert_eq!(
            effective_url("http://localhost:8080/api/ocr"),
            "http://localhost:8080/api/ocr"
        );
    }
}
