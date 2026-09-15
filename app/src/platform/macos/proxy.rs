//! 系统代理（design §2.5，R5）：解析 `scutil --proxy` 的输出，结果缓存 30 秒。
//! 只认手动 HTTP / HTTPS 代理；PAC 自动配置和 SOCKS 不支持。

use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::platform::{bypass, process};

const SCUTIL: &str = "/usr/sbin/scutil";
const CACHE_FOR: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Default, PartialEq)]
struct Settings {
    http: Option<String>,
    https: Option<String>,
    exceptions: Vec<String>,
    exclude_simple: bool,
}

static CACHE: Mutex<Option<(Instant, Settings)>> = Mutex::new(None);

pub fn system_proxy(url: &str) -> Option<String> {
    let (scheme, host) = bypass::target(url)?;
    if bypass::is_loopback(&host) {
        return None;
    }
    choose(&settings(), &scheme, &host)
}

fn choose(s: &Settings, scheme: &str, host: &str) -> Option<String> {
    let simple = !host.contains('.') && !host.contains(':');
    if (s.exclude_simple && simple) || s.exceptions.iter().any(|p| bypass::matches(host, p)) {
        return None;
    }
    if scheme == "https" {
        s.https.clone()
    } else {
        s.http.clone()
    }
}

fn settings() -> Settings {
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((at, s)) = cache.as_ref()
        && at.elapsed() < CACHE_FOR
    {
        return s.clone();
    }
    let s = match process::run(
        Path::new(SCUTIL),
        &["--proxy".into()],
        Duration::from_secs(3),
    ) {
        Ok(out) => parse(&String::from_utf8_lossy(&out.stdout)),
        Err(e) => {
            log::warn!("Proxy: scutil failed: {e}");
            Settings::default()
        }
    };
    *cache = Some((Instant::now(), s.clone()));
    s
}

/// ```text
/// <dictionary> {
///   ExceptionsList : <array> {
///     0 : *.local
///   }
///   HTTPEnable : 1
///   HTTPPort : 7890
///   HTTPProxy : 127.0.0.1
/// }
/// ```
fn parse(text: &str) -> Settings {
    let mut values = Vec::new();
    let mut exceptions = Vec::new();
    let mut in_list = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with("ExceptionsList") {
            in_list = true;
        } else if in_list && line == "}" {
            in_list = false;
        } else if let Some((key, value)) = line.split_once(" : ") {
            if in_list {
                exceptions.push(value.trim().to_owned());
            } else {
                values.push((key.trim(), value.trim()));
            }
        }
    }
    let get = |key: &str| values.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
    let proxy = |prefix: &str| {
        if get(&format!("{prefix}Enable")) != Some("1") {
            return None;
        }
        let host = get(&format!("{prefix}Proxy"))?;
        let port = get(&format!("{prefix}Port")).unwrap_or("80");
        bypass::proxy_uri(&format!("{host}:{port}"))
    };
    Settings {
        http: proxy("HTTP"),
        https: proxy("HTTPS"),
        exceptions,
        exclude_simple: get("ExcludeSimpleHostnames") == Some("1"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLASH: &str = "<dictionary> {
  ExceptionsList : <array> {
    0 : 127.0.0.1
    1 : localhost
    2 : *.local
    3 : 169.254/16
  }
  ExcludeSimpleHostnames : 1
  HTTPEnable : 1
  HTTPPort : 7890
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 7890
  HTTPSProxy : 127.0.0.1
  SOCKSEnable : 1
  SOCKSPort : 7891
  SOCKSProxy : 127.0.0.1
}";

    #[test]
    fn parses_clash_settings() {
        let s = parse(CLASH);
        assert_eq!(s.http.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(s.https.as_deref(), Some("http://127.0.0.1:7890"));
        assert_eq!(s.exceptions.len(), 4);
        assert!(s.exclude_simple);
        assert_eq!(
            choose(&s, "https", "translate.google.com").as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(choose(&s, "http", "printer.local"), None);
        assert_eq!(choose(&s, "http", "intranet"), None);
    }

    #[test]
    fn disabled_or_empty_means_direct() {
        let s = parse("<dictionary> {\n  HTTPEnable : 0\n  HTTPProxy : p\n  HTTPPort : 1\n}");
        assert_eq!(s, Settings::default());
        assert_eq!(choose(&parse(""), "https", "example.com"), None);
    }
}
