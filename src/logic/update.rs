//! 检查更新（F8）。只告知，不自动下载安装（旧版 About/index.jsx 同款）。
//! 不走 api.github.com（未登录易被限流 403），HEAD releases/latest 跟随重定向看最终地址。

use std::time::Duration;

use crate::error::{Error, HttpKind};
use crate::service::http;

const TIMEOUT: Duration = Duration::from_secs(10);
const RELEASES_LATEST: &str = "https://github.com/BearKouber/TiLex/releases/latest";

/// 检查更新结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Release {
    /// 存在新版本（附带版本 tag，如 "v0.2.0"）。
    Newer(String),
    /// 已是最新版本。
    Latest,
    /// 仓库未发布任何版本（重定向到 /releases 或返回 404）。
    None,
}

/// 执行检查更新（通过网络请求 GitHub releases/latest）。
pub fn check() -> Result<Release, Error> {
    let (status, final_url) = http::final_url(RELEASES_LATEST, TIMEOUT)?;
    classify(status, &final_url, env!("CARGO_PKG_VERSION"))
}

/// 根据 HTTP 响应状态码、跟随重定向后的最终 URL 以及当前版本号，分类版本状态。
pub fn classify(status: u16, final_url: &str, current: &str) -> Result<Release, Error> {
    if status == 404 {
        return Ok(Release::None);
    }
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

    let Some(tag) = extract_tag(final_url) else {
        return Ok(Release::None);
    };

    if is_newer(&tag, current) {
        Ok(Release::Newer(tag))
    } else {
        Ok(Release::Latest)
    }
}

/// 从形如 `.../releases/tag/<tag>` 的 URL 中提取 tag 字符串。
fn extract_tag(url: &str) -> Option<String> {
    let base = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim_end_matches('/');
    let (_, tag) = base.rsplit_once("/releases/tag/")?;
    if tag.is_empty() || tag.contains('/') {
        return None;
    }
    Some(tag.to_string())
}

/// 比较 tag 版本是否比当前版本 current 更具更新（旧版 isNewer 逻辑）。
/// 去掉开头的 'v' 或 'V'，最多比较前 3 段数字，非数字后缀转为 0。
pub fn is_newer(tag: &str, current: &str) -> bool {
    let a = tag.strip_prefix(['v', 'V']).unwrap_or(tag);
    let b = current.strip_prefix(['v', 'V']).unwrap_or(current);
    let mut a_parts = a.split('.');
    let mut b_parts = b.split('.');

    for _ in 0..3 {
        let x = parse_part(a_parts.next());
        let y = parse_part(b_parts.next());
        if x != y {
            return x > y;
        }
    }
    false
}

fn parse_part(s: Option<&str>) -> u32 {
    let Some(s) = s else { return 0 };
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_cases() {
        // 新版本
        assert!(is_newer("v0.2.0", "0.1.1"));
        assert!(is_newer("0.2.0", "0.1.1"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(is_newer("0.1.2", "0.1.1"));

        // 同版本
        assert!(!is_newer("v0.1.1", "0.1.1"));
        assert!(!is_newer("0.1.1", "0.1.1"));

        // 更旧版本
        assert!(!is_newer("v0.1.0", "0.1.1"));
        assert!(!is_newer("0.0.9", "0.1.1"));

        // v 前缀
        assert!(is_newer("v0.2.0", "v0.1.1"));
        assert!(!is_newer("v0.1.1", "v0.1.1"));
        assert!(!is_newer("v0.1.0", "v0.1.1"));

        // 包含 beta 等非数字后缀
        assert!(is_newer("0.2.0-beta", "0.1.1"));
        assert!(!is_newer("0.1.1-beta", "0.1.1"));
        assert!(!is_newer("0.1.0-beta", "0.1.1"));
    }

    #[test]
    fn test_classify_newer() {
        let url = "https://github.com/BearKouber/TiLex/releases/tag/v0.2.0";
        let res = classify(200, url, "0.1.1").unwrap();
        assert_eq!(res, Release::Newer("v0.2.0".into()));
    }

    #[test]
    fn test_classify_latest() {
        let url = "https://github.com/BearKouber/TiLex/releases/tag/v0.1.1";
        let res = classify(200, url, "0.1.1").unwrap();
        assert_eq!(res, Release::Latest);
    }

    #[test]
    fn test_classify_older() {
        let url = "https://github.com/BearKouber/TiLex/releases/tag/v0.1.0";
        let res = classify(200, url, "0.1.1").unwrap();
        assert_eq!(res, Release::Latest);
    }

    #[test]
    fn test_classify_no_release_releases_url() {
        let url = "https://github.com/BearKouber/TiLex/releases";
        let res = classify(200, url, "0.1.1").unwrap();
        assert_eq!(res, Release::None);

        let url_slash = "https://github.com/BearKouber/TiLex/releases/";
        let res_slash = classify(200, url_slash, "0.1.1").unwrap();
        assert_eq!(res_slash, Release::None);
    }

    #[test]
    fn test_classify_no_release_404() {
        let url = "https://github.com/BearKouber/TiLex/releases/latest";
        let res = classify(404, url, "0.1.1").unwrap();
        assert_eq!(res, Release::None);
    }

    #[test]
    fn test_classify_server_error_500() {
        let url = "https://github.com/BearKouber/TiLex/releases/latest";
        let res = classify(500, url, "0.1.1");
        match res {
            Err(Error::Http { status, kind }) => {
                assert_eq!(status, Some(500));
                assert_eq!(kind, HttpKind::Server);
            }
            other => panic!("expected Error::Http, got {other:?}"),
        }
    }
}
