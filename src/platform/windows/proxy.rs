//! 系统代理（design §2.5，R5）：每次请求读 `HKCU\...\Internet Settings`，改了 Clash 不用重启。
//! 只认手动代理（`ProxyEnable` / `ProxyServer` / `ProxyOverride`）；PAC 自动配置脚本不支持（旧版 reqwest 也不支持）。

use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::core::{PCWSTR, w};

use crate::platform::bypass;

const KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Internet Settings");

pub fn system_proxy(url: &str) -> Option<String> {
    let (scheme, host) = bypass::target(url)?;
    if bypass::is_loopback(&host) || read_dword(w!("ProxyEnable"))? == 0 {
        return None;
    }
    if bypassed(&host, &read_string(w!("ProxyOverride")).unwrap_or_default()) {
        return None;
    }
    choose(&read_string(w!("ProxyServer"))?, &scheme)
}

/// `ProxyServer` 两种写法：`host:port`（所有协议共用，Clash 就是这种），
/// 或 `http=a:1;https=b:2;socks=c:3`（按协议分）。https 没有单独一项时借用 http 那项。
/// ponytail: 只走 HTTP 代理；只配了 `socks=` 的当没开代理（ureq 没开 socks 特性）。
fn choose(server: &str, scheme: &str) -> Option<String> {
    if !server.contains('=') {
        return bypass::proxy_uri(server);
    }
    let entry = |name: &str| {
        server.split(';').find_map(|part| {
            let (key, value) = part.split_once('=')?;
            key.trim().eq_ignore_ascii_case(name).then_some(value)
        })
    };
    let picked = if scheme == "https" {
        entry("https").or_else(|| entry("http"))
    } else {
        entry("http")
    };
    picked.and_then(bypass::proxy_uri)
}

/// `ProxyOverride`：分号分隔的通配项；`<local>` 表示不带点的主机名直连。
fn bypassed(host: &str, list: &str) -> bool {
    list.split(';').any(|pattern| {
        if pattern.trim().eq_ignore_ascii_case("<local>") {
            !host.contains('.') && !host.contains(':')
        } else {
            bypass::matches(host, pattern)
        }
    })
}

fn read_dword(name: PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: KEY、name 是以 0 结尾的常量；value / size 在调用期间有效，size 与 value 大小一致。
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            KEY,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut value).cast()),
            Some(&raw mut size),
        )
    };
    (result == ERROR_SUCCESS).then_some(value)
}

fn read_string(name: PCWSTR) -> Option<String> {
    let mut size = 0u32;
    // SAFETY: 只查长度，数据指针为空；size 在调用期间有效。
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            KEY,
            name,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&raw mut size),
        )
    };
    if result != ERROR_SUCCESS || size == 0 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    // SAFETY: buf 有 size 字节；两次调用之间值变长时返回 ERROR_MORE_DATA，按没读到处理。
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            KEY,
            name,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&raw mut size),
        )
    };
    if result != ERROR_SUCCESS {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_address_serves_every_scheme() {
        assert_eq!(
            choose("127.0.0.1:7890", "https").as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            choose("127.0.0.1:7890", "http").as_deref(),
            Some("http://127.0.0.1:7890")
        );
    }

    #[test]
    fn per_scheme_entries() {
        let server = "http=127.0.0.1:8080;https=127.0.0.1:8443;socks=127.0.0.1:1080";
        assert_eq!(
            choose(server, "https").as_deref(),
            Some("http://127.0.0.1:8443")
        );
        assert_eq!(
            choose(server, "http").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(
            choose("HTTP=p:1", "https").as_deref(),
            Some("http://p:1"),
            "https 借用 http 那项"
        );
        assert_eq!(choose("socks=127.0.0.1:1080", "https"), None);
        assert_eq!(choose("https=p:1", "http"), None);
    }

    #[test]
    fn override_list() {
        let list = "localhost;127.*;10.*;*.corp.example;<local>";
        assert!(bypassed("10.2.3.4", list));
        assert!(bypassed("wiki.corp.example", list));
        assert!(bypassed("intranet", list), "<local> = 不带点的主机名");
        assert!(!bypassed("translate.google.com", list));
        assert!(!bypassed("intranet", "*.lan"));
    }

    // 读本机真实注册表：只断言不崩、回环永远直连。
    #[test]
    fn live_registry_never_proxies_loopback() {
        assert_eq!(system_proxy("http://127.0.0.1:1224/api/ocr"), None);
        assert_eq!(system_proxy("http://localhost:8045/v1"), None);
        let _ = system_proxy("https://translate.google.com/"); // ignore: 结果取决于本机设置
    }
}
