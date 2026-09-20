//! 系统代理的公共部分：从地址取主机、回环判断、例外列表匹配。两个平台的 `proxy.rs` 共用。

/// `https://user@host:443/path` → `("https", "host")`。IPv6 字面量去掉方括号。取不出返回 `None`。
pub fn target(url: &str) -> Option<(String, String)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    let host = if let Some(v6) = host_port.strip_prefix('[') {
        v6.split(']').next()?
    } else {
        host_port.split(':').next()?
    };
    if host.is_empty() {
        return None;
    }
    Some((scheme.to_ascii_lowercase(), host.to_ascii_lowercase()))
}

/// 回环地址永远直连（design §2.5）：本机的 Umi-OCR、本地模型网关不能绕到代理上去。
pub fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host.ends_with(".localhost")
        || host == "::1"
        || host
            .parse::<std::net::Ipv4Addr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// 例外列表的一项：大小写不敏感，只认 `*` 通配（`*.lan`、`10.*`、`192.168.1.*`）。
/// ponytail: 不认 CIDR（例如 `169.254/16`），要时再加。
pub fn matches(host: &str, pattern: &str) -> bool {
    let pattern = pattern.trim().to_ascii_lowercase();
    if pattern.is_empty() {
        return false;
    }
    glob(host.to_ascii_lowercase().as_bytes(), pattern.as_bytes())
}

fn glob(text: &[u8], pattern: &[u8]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) => (0..=text.len()).any(|i| glob(&text[i..], rest)),
        Some((c, rest)) => text.first() == Some(c) && glob(&text[1..], rest),
    }
}

/// 代理地址没写协议就当 HTTP 代理。
pub fn proxy_uri(address: &str) -> Option<String> {
    let address = address.trim();
    if address.is_empty() {
        None
    } else if address.contains("://") {
        Some(address.to_owned())
    } else {
        Some(format!("http://{address}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_takes_scheme_and_host() {
        let t = |u: &str| target(u).map(|(s, h)| format!("{s} {h}"));
        assert_eq!(
            t("https://translate.google.com/translate_a/single?q=1").as_deref(),
            Some("https translate.google.com")
        );
        assert_eq!(
            t("http://user:pw@127.0.0.1:8045/v1").as_deref(),
            Some("http 127.0.0.1")
        );
        assert_eq!(t("HTTP://[::1]:1224/api").as_deref(), Some("http ::1"));
        assert_eq!(t("API.Example.com").as_deref(), None);
        assert_eq!(t("https:///path"), None);
    }

    #[test]
    fn loopback_is_always_direct() {
        for host in [
            "localhost",
            "127.0.0.1",
            "127.8.9.1",
            "::1",
            "app.localhost",
        ] {
            assert!(is_loopback(host), "{host}");
        }
        for host in [
            "10.0.0.1",
            "example.com",
            "localhost.example.com",
            "128.0.0.1",
        ] {
            assert!(!is_loopback(host), "{host}");
        }
    }

    #[test]
    fn wildcard_patterns() {
        assert!(matches("printer.lan", "*.lan"));
        assert!(matches("10.1.2.3", "10.*"));
        assert!(matches("Example.COM", "example.com"));
        assert!(matches("a.b.corp.net", "*.corp.*"));
        assert!(!matches("lan", "*.lan"));
        assert!(!matches("example.com", "example.co"));
        assert!(!matches("example.com", " "));
    }

    #[test]
    fn proxy_address_defaults_to_http() {
        assert_eq!(
            proxy_uri("127.0.0.1:7890").as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            proxy_uri(" http://p:8080 ").as_deref(),
            Some("http://p:8080")
        );
        assert_eq!(proxy_uri("  "), None);
    }
}
