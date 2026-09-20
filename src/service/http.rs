//! HTTP（design §2.5，D14）。全项目唯一用 ureq 的文件（R-4，source_rules 检查）。
//! 每个函数超时必填；响应体一次最多读 `MAX_BODY_BYTES`；错误只留分类和状态码，不带响应体和地址（可能回显 key）。
//! 代理每次请求时向平台层查系统设置，回环地址直连。

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;
use ureq::http::Response;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::{Agent, Body, Proxy, ResponseExt};

use crate::error::{Error, HttpKind};
use crate::platform;

/// 普通翻译。
pub const TIMEOUT_TRANSLATE: Duration = Duration::from_secs(15);
/// 在线语种检测（niutrans / baidu / google）。
pub const TIMEOUT_DETECT: Duration = Duration::from_secs(5);
/// AI 翻译：模型出一段 JSON 可能要几十秒。
pub const TIMEOUT_AI: Duration = Duration::from_secs(60);

/// 单次响应体的硬上限。ureq 默认 10 MiB，这里压到 4 MiB：全项目只有 `finish` 一处读响应体，
/// AI 的模型列表（`service/ai/protocol.rs`）也走这里，几百 KB 量级 —— 4 MiB 比任何已知合法响应大一个数量级，
/// 同时把病态响应一次能吃的内存砍掉 60%。超限算 `Format`，见 `classify`。
const MAX_BODY_BYTES: u64 = 4 * 1024 * 1024;

/// GET。`query` 按顺序追加并做 URL 编码（同名键可以重复）。2xx 的响应体能解析成 JSON 就返回 JSON，
/// 否则原样作为 `Value::String` 返回（有的中转接口直接回纯文本）；`trim_start` 后以 `<` 开头的当格式错误，
/// 细节见 `parse_body`。响应体最多读 `MAX_BODY_BYTES`。
pub fn get(
    url: &str,
    query: &[(&str, &str)],
    headers: &[(&str, &str)],
    timeout: Duration,
) -> Result<Value, Error> {
    let mut req = agent().get(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let req = req.query_pairs(query.iter().copied());
    finish(
        req.config()
            .timeout_global(Some(timeout))
            .proxy(proxy(url))
            .build()
            .call(),
    )
}

/// POST JSON（自动加 `Content-Type: application/json`）。返回值同 [`get`]。
pub fn post_json(
    url: &str,
    headers: &[(&str, &str)],
    body: &Value,
    timeout: Duration,
) -> Result<Value, Error> {
    let bytes = serde_json::to_vec(body)?;
    let mut req = agent().post(url).header("Content-Type", "application/json");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    finish(
        req.config()
            .timeout_global(Some(timeout))
            .proxy(proxy(url))
            .build()
            .send(&bytes[..]),
    )
}

/// POST 文本（自动加 `Content-Type: application/json`）。直接发字符串 body，返回值同 [`get`]。
pub fn post_text(
    url: &str,
    headers: &[(&str, &str)],
    body: &str,
    timeout: Duration,
) -> Result<Value, Error> {
    let mut req = agent().post(url).header("Content-Type", "application/json");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    finish(
        req.config()
            .timeout_global(Some(timeout))
            .proxy(proxy(url))
            .build()
            .send(body.as_bytes()),
    )
}

/// POST 表单（`application/x-www-form-urlencoded`）。返回值同 [`get`]。
pub fn post_form(
    url: &str,
    headers: &[(&str, &str)],
    form: &[(&str, &str)],
    timeout: Duration,
) -> Result<Value, Error> {
    let mut req = agent().post(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    finish(
        req.config()
            .timeout_global(Some(timeout))
            .proxy(proxy(url))
            .build()
            .send_form(form.iter().copied()),
    )
}

/// HEAD，跟随重定向，返回最终状态码和最终地址（[`ResponseExt::get_uri`])。
/// 404 不当错误返回；其他非 2xx 照现有函数错误分类。
pub fn final_url(url: &str, timeout: Duration) -> Result<(u16, String), Error> {
    let req = agent().head(url);
    let resp = req
        .config()
        .timeout_global(Some(timeout))
        .proxy(proxy(url))
        .build()
        .call()
        .map_err(classify)?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) && status != 404 {
        let kind = if (400..500).contains(&status) {
            HttpKind::Client
        } else {
            HttpKind::Server
        };
        return Err(Error::Http {
            status: Some(status),
            kind,
        });
    }
    let final_uri = resp.get_uri().to_string();
    Ok((status, final_uri))
}

/// 进程内一个 Agent（连接池复用）。状态码自己判断；不读环境变量里的代理，代理按请求设。
fn agent() -> &'static Agent {
    static AGENT: OnceLock<Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::Rustls)
            .root_certs(RootCerts::PlatformVerifier)
            .build();
        Agent::config_builder()
            .http_status_as_error(false)
            .proxy(None)
            .tls_config(tls)
            .build()
            .new_agent()
    })
}

fn proxy(url: &str) -> Option<Proxy> {
    let uri = platform::system_proxy(url)?;
    match Proxy::new(&uri) {
        Ok(p) => Some(p),
        Err(_) => {
            // 不记地址：代理地址里可能带用户名密码。
            log::warn!("Http: system proxy not usable, going direct");
            None
        }
    }
}

fn finish(result: Result<Response<Body>, ureq::Error>) -> Result<Value, Error> {
    let mut resp = result.map_err(classify)?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        let kind = if (400..500).contains(&status) {
            HttpKind::Client
        } else {
            HttpKind::Server
        };
        return Err(Error::Http {
            status: Some(status),
            kind,
        });
    }
    let body = resp
        .body_mut()
        .with_config()
        .limit(MAX_BODY_BYTES)
        .read_to_string()
        .map_err(classify)?;
    parse_body(body)
}

/// 响应体文本 → `Value`。能解析成 JSON 就返回 JSON；否则按规范原样作为 `Value::String` 返回（有的中转接口回纯文本）。
///
/// 例外：解析不了且 `trim_start` 后以 `<` 开头（HTML / XML 错误页）→ `Format`。中转站、镜像站挂掉时常回一页 HTML，
/// 当译文返回的代价是用户看到满屏标签，而且被写进结果缓存（200 条）里，同一句再划还是 HTML。
///
/// 天花板：只认得 `<` 开头的错误页，纯文本的 `502 Bad Gateway` 之类仍会当译文显示（短，危害小，不再加规则）。
/// 错误里不带响应体（可能回显 key），所以这里不把 `body` 拼进错误。
///
/// 反过来的误伤：纯文本中转接口回的译文自己以 `<` 开头（源文本是标签）也会被判 `Format`，比错误页少见得多。
fn parse_body(body: String) -> Result<Value, Error> {
    if let Ok(value) = serde_json::from_str(&body) {
        return Ok(value);
    }
    if body.trim_start().starts_with('<') {
        return Err(Error::Http {
            status: None,
            kind: HttpKind::Format,
        });
    }
    Ok(Value::String(body))
}

fn classify(e: ureq::Error) -> Error {
    use ureq::Error as E;
    let kind = match &e {
        E::Timeout(_) => HttpKind::Timeout,
        // 响应体超过 `MAX_BODY_BYTES`：内容没读全，是形状问题，不是连不上。
        E::BodyExceedsLimit(_) => HttpKind::Format,
        E::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => HttpKind::Timeout,
        // 超限也可能以 `io::Error` 的形状到这儿（`Error::into_io` 包一层，`ErrorKind::Other`），
        // 和「连不上」用 kind 分不开，只能看里面装的是什么。
        E::Io(io) => {
            if is_body_exceeds_limit(io) {
                HttpKind::Format
            } else {
                HttpKind::Connect
            }
        }
        E::StatusCode(code) => {
            return Error::Http {
                status: Some(*code),
                kind: HttpKind::Server,
            };
        }
        E::HostNotFound
        | E::ConnectionFailed
        | E::ConnectProxyFailed(_)
        | E::InvalidProxyUrl
        | E::BadUri(_)
        | E::Tls(_)
        | E::Rustls(_) => HttpKind::Connect,
        _ => HttpKind::Format,
    };
    Error::Http { status: None, kind }
}

/// 这个 `io::Error` 里装的是不是 ureq 的「响应体超限」。只看类型，不匹配错误字符串。
fn is_body_exceeds_limit(io: &std::io::Error) -> bool {
    io.get_ref()
        .and_then(|inner| inner.downcast_ref::<ureq::Error>())
        .is_some_and(|e| matches!(e, ureq::Error::BodyExceedsLimit(_)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread::{self, JoinHandle};
    use std::time::Instant;

    /// 本机起一个只接一次连接的服务器：收完请求（按 Content-Length）回 `response`，返回收到的请求原文。
    /// 回环地址不走代理，本机开着 Clash 也不影响。
    fn serve_owned(response: String, delay: Duration) -> (String, JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let n = stream.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if buf.len() >= end + 4 + length {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            thread::sleep(delay);
            let _ = stream.write_all(response.as_bytes());
            String::from_utf8_lossy(&buf).to_string()
        });
        (url, handle)
    }

    /// 同上，响应体是常量字符串（大多数测试够用）。
    fn serve(response: &'static str, delay: Duration) -> (String, JoinHandle<String>) {
        serve_owned(response.to_string(), delay)
    }

    const OK_JSON: &str = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 12\r\nConnection: close\r\n\r\n{\"ok\":[1,2]}";

    #[test]
    fn get_parses_json_and_encodes_query() {
        let (url, server) = serve(OK_JSON, Duration::ZERO);
        let value = get(
            &format!("{url}/translate_a/single"),
            &[("dt", "at"), ("dt", "t"), ("q", "a b&c")],
            &[("X-Test", "1")],
            TIMEOUT_TRANSLATE,
        )
        .unwrap();
        assert_eq!(value, serde_json::json!({"ok": [1, 2]}));
        let request = server.join().unwrap();
        let line = request.lines().next().unwrap();
        assert!(
            line.starts_with("GET /translate_a/single?dt=at&dt=t&q="),
            "{line}"
        );
        assert!(!line.contains("a b&c"), "query must be encoded: {line}");
        assert!(request.to_ascii_lowercase().contains("x-test: 1"));
    }

    #[test]
    fn non_json_body_is_returned_as_text() {
        let (url, server) = serve(
            "HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello",
            Duration::ZERO,
        );
        assert_eq!(
            get(&url, &[], &[], TIMEOUT_TRANSLATE).unwrap(),
            Value::String("hello".into())
        );
        server.join().unwrap();
    }

    #[test]
    fn json_body_is_parsed() {
        assert_eq!(
            parse_body(r#"{"ok":[1,2]}"#.to_string()).unwrap(),
            serde_json::json!({"ok": [1, 2]})
        );
    }

    #[test]
    fn plain_text_body_is_returned_as_text() {
        assert_eq!(
            parse_body("hello".to_string()).unwrap(),
            Value::String("hello".into())
        );
    }

    #[test]
    fn text_body_with_angle_bracket_is_not_html() {
        // `<` 不在开头（纯文本译文里出现标签）仍按纯文本返回：判定只看第一个非空白字符。
        assert_eq!(
            parse_body("a < b".to_string()).unwrap(),
            Value::String("a < b".into())
        );
    }

    #[test]
    fn html_body_is_a_format_error() {
        let body =
            r#"<!doctype html><html><body><h1>502 Bad Gateway</h1></body></html>"#.to_string();
        let err = parse_body(body).unwrap_err();
        assert!(
            matches!(
                err,
                Error::Http {
                    status: None,
                    kind: HttpKind::Format
                }
            ),
            "{err:?}"
        );
        let shown = format!("{err} {err:?}");
        assert!(!shown.contains("html"), "错误里不许带响应体：{shown}");
    }

    #[test]
    fn html_body_after_whitespace_is_a_format_error() {
        let body = format!("\n  {}", r#"<html lang="en">err</html>"#);
        let err = parse_body(body).unwrap_err();
        assert!(
            matches!(
                err,
                Error::Http {
                    status: None,
                    kind: HttpKind::Format
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn body_exceeds_limit_is_a_format_error() {
        // 超限不许落进 `E::Io(_)` 那一臂被当成「连不上」。
        let plain = classify(ureq::Error::BodyExceedsLimit(MAX_BODY_BYTES));
        assert!(
            matches!(
                plain,
                Error::Http {
                    status: None,
                    kind: HttpKind::Format
                }
            ),
            "{plain:?}"
        );
        // 包成 io::Error（`ErrorKind::Other`）也要认得出来。
        let wrapped = classify(ureq::Error::Io(
            ureq::Error::BodyExceedsLimit(MAX_BODY_BYTES).into_io(),
        ));
        assert!(
            matches!(
                wrapped,
                Error::Http {
                    status: None,
                    kind: HttpKind::Format
                }
            ),
            "{wrapped:?}"
        );
    }

    #[test]
    fn oversized_response_body_is_a_format_error() {
        // 5 MiB > `MAX_BODY_BYTES`：走真实读取路径，确认真读超限时出来的是 `Format`（不是 `Connect`）。
        let filler = "x".repeat(5 * 1024 * 1024);
        let (url, server) = serve_owned(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{filler}",
                filler.len()
            ),
            Duration::ZERO,
        );
        let result = get(&url, &[], &[], TIMEOUT_TRANSLATE);
        let shape = match &result {
            Ok(v) => format!("Ok(len={})", v.as_str().map_or(0, str::len)),
            Err(e) => format!("Err({e})"),
        };
        assert!(
            matches!(
                result,
                Err(Error::Http {
                    status: None,
                    kind: HttpKind::Format
                })
            ),
            "{shape}"
        );
        server.join().unwrap();
    }

    #[test]
    fn truncated_body_is_a_connect_error() {
        // 服务端只发一半就断：坏掉的 body 读取里，真网络故障仍归 `Connect`，别被超限那套一起吃掉。
        let (url, server) = serve(
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\nhalf",
            Duration::ZERO,
        );
        let result = get(&url, &[], &[], TIMEOUT_TRANSLATE);
        assert!(
            matches!(
                result,
                Err(Error::Http {
                    status: None,
                    kind: HttpKind::Connect
                })
            ),
            "{result:?}"
        );
        server.join().unwrap();
    }

    #[test]
    fn post_json_sends_body_and_content_type() {
        let (url, server) = serve(OK_JSON, Duration::ZERO);
        let body = serde_json::json!({"text": "中文 \"quoted\""});
        post_json(&url, &[("x-api-key", "k")], &body, TIMEOUT_AI).unwrap();
        let request = server.join().unwrap();
        assert!(request.starts_with("POST / "));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: application/json")
        );
        assert!(request.ends_with(&body.to_string()), "{request}");
    }

    #[test]
    fn post_text_sends_body_and_content_type() {
        let (url, server) = serve(OK_JSON, Duration::ZERO);
        let body = "{\"method\" : \"LMT_handle_texts\"}";
        post_text(&url, &[("x-api-key", "k")], body, TIMEOUT_TRANSLATE).unwrap();
        let request = server.join().unwrap();
        assert!(request.starts_with("POST / "));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("content-type: application/json")
        );
        assert!(request.ends_with(body), "{request}");
    }

    #[test]
    fn post_form_is_urlencoded() {
        let (url, server) = serve(OK_JSON, Duration::ZERO);
        post_form(&url, &[], &[("query", "a b")], TIMEOUT_TRANSLATE).unwrap();
        let request = server.join().unwrap();
        assert!(
            request
                .to_ascii_lowercase()
                .contains("application/x-www-form-urlencoded")
        );
        assert!(request.ends_with("query=a+b") || request.ends_with("query=a%20b"));
    }

    #[test]
    fn error_status_keeps_only_the_code() {
        let (url, server) = serve(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 20\r\nConnection: close\r\n\r\n{\"key\":\"sk-secret1\"}",
            Duration::ZERO,
        );
        let err = get(&url, &[], &[], TIMEOUT_TRANSLATE).unwrap_err();
        assert!(matches!(
            err,
            Error::Http {
                status: Some(401),
                kind: HttpKind::Client
            }
        ));
        let shown = format!("{err} {err:?}");
        assert!(
            !shown.contains("sk-secret1") && !shown.contains("127.0.0.1"),
            "{shown}"
        );
        server.join().unwrap();

        let (url, server) = serve(
            "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            Duration::ZERO,
        );
        assert!(matches!(
            get(&url, &[], &[], TIMEOUT_TRANSLATE),
            Err(Error::Http {
                status: Some(502),
                kind: HttpKind::Server
            })
        ));
        server.join().unwrap();
    }

    #[test]
    fn silent_server_times_out() {
        let (url, server) = serve(OK_JSON, Duration::from_secs(3));
        let started = Instant::now();
        let result = get(&url, &[], &[], Duration::from_millis(300));
        assert!(
            matches!(
                result,
                Err(Error::Http {
                    kind: HttpKind::Timeout,
                    ..
                })
            ),
            "{result:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }

    #[test]
    fn closed_port_is_a_connect_error() {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let result = get(
            &format!("http://127.0.0.1:{port}/"),
            &[],
            &[],
            TIMEOUT_TRANSLATE,
        );
        assert!(
            matches!(
                result,
                Err(Error::Http {
                    kind: HttpKind::Connect,
                    ..
                })
            ),
            "{result:?}"
        );
    }
}
