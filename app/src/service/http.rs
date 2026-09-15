//! HTTP（design §2.5，D14）。全项目唯一用 ureq 的文件（R-4，source_rules 检查）。
//! 每个函数超时必填；错误只留分类和状态码，不带响应体和地址（可能回显 key）。
//! 代理每次请求时向平台层查系统设置，回环地址直连。

use std::sync::OnceLock;
use std::time::Duration;

use serde_json::Value;
use ureq::http::Response;
use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::{Agent, Body, Proxy, ResponseExt};

use crate::error::{Error, HttpKind};
use crate::platform;

/// 普通翻译、语种检测。
pub const TIMEOUT_TRANSLATE: Duration = Duration::from_secs(15);
/// AI 翻译：模型出一段 JSON 可能要几十秒。
pub const TIMEOUT_AI: Duration = Duration::from_secs(60);

/// GET。`query` 按顺序追加并做 URL 编码（同名键可以重复）。2xx 的响应体能解析成 JSON 就返回 JSON，
/// 否则原样作为 `Value::String` 返回（有的中转接口直接回纯文本）。
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
    let body = resp.body_mut().read_to_string().map_err(classify)?;
    Ok(serde_json::from_str(&body).unwrap_or(Value::String(body)))
}

fn classify(e: ureq::Error) -> Error {
    use ureq::Error as E;
    let kind = match &e {
        E::Timeout(_) => HttpKind::Timeout,
        E::Io(io) if io.kind() == std::io::ErrorKind::TimedOut => HttpKind::Timeout,
        E::StatusCode(code) => {
            return Error::Http {
                status: Some(*code),
                kind: HttpKind::Server,
            };
        }
        E::HostNotFound
        | E::ConnectionFailed
        | E::Io(_)
        | E::ConnectProxyFailed(_)
        | E::InvalidProxyUrl
        | E::BadUri(_)
        | E::Tls(_)
        | E::Rustls(_) => HttpKind::Connect,
        _ => HttpKind::Format,
    };
    Error::Http { status: None, kind }
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
    fn serve(response: &'static str, delay: Duration) -> (String, JoinHandle<String>) {
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
