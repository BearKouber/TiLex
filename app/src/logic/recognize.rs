//! 识别服务调度（prd F4）。
//!
//! 只取识别列表里第一个启用的服务识别图片。后台线程调用，不阻塞 UI 线程（R-5）。

use std::path::Path;

use crate::error::{Error, HttpKind};
use crate::logic::config::{self, Service};
use crate::platform;
use crate::service::{self, umi};

/// 从识别服务列表中挑出第一个启用的识别服务。
///
/// 认不出的服务（`Unknown`）以及误入识别列表的非识别服务（如翻译服务）均会被跳过。
fn first_enabled(services: &[Service]) -> Option<&Service> {
    services.iter().find(|s| match s {
        Service::Wechat(i) => i.enabled,
        Service::Apple(i) => i.enabled,
        Service::Umi(i) => i.enabled,
        _ => false,
    })
}

fn service_name(service: &Service) -> Option<&'static str> {
    match service {
        Service::Wechat(_) => Some("wechat"),
        Service::Apple(_) => Some("apple"),
        Service::Umi(_) => Some("umi"),
        _ => None,
    }
}

/// 第一个启用的识别服务叫什么（界面报错时要说是哪家）。没有就是 `None`。
pub fn current() -> Option<&'static str> {
    let config = config::snapshot();
    let service = first_enabled(&config.recognize_services)?;
    service_name(service)
}

/// 用识别列表里**第一个启用的**服务识别一张图（prd F4）。
/// 后台线程调，几百毫秒到几秒（R-5）。
/// 一个启用的服务都没有时返回 `Error::NotConfigured("recognize_services")`。
pub fn run(image: &Path) -> Result<String, Error> {
    let config = config::snapshot();
    let Some(service) = first_enabled(&config.recognize_services) else {
        return Err(Error::NotConfigured("recognize_services"));
    };
    let name = service_name(service).unwrap_or("unknown");
    let result = match service {
        Service::Wechat(_) => platform::wechat_ocr(image),
        Service::Apple(_) => platform::apple_ocr(image),
        Service::Umi(i) => service::umi::recognize(&i.config, image),
        _ => return Err(Error::NotConfigured("recognize_services")),
    };
    match &result {
        Ok(text) => log::info!("Recognize: {name} {} chars", text.chars().count()),
        Err(e) => log::warn!("Recognize: {name} failed: {e}"),
    }
    result
}

/// 识别一张图，结果折成界面要的形状：出了字给文字，没出字给个编号让 `.slint` 选文案
/// （Rust 不拼界面中文，design §2.9）。编号见 [`failure_code`]。后台线程调（R-5）。
pub fn run_for_ui(image: &Path) -> Result<String, i32> {
    let service = current();
    let result = run(image);
    match failure_code(&result, service) {
        0 => result.map_err(|_| FAILED),
        code => Err(code),
    }
}

/// 图里没有文字（微信 OCR 一个非空块都没有 / Umi 的 code 101）。
pub const NO_TEXT: i32 = 1;
/// 识别服务列表里一个启用的都没有。
pub const NO_SERVICE: i32 = 2;
/// Umi-OCR 连不上，多半是用户没启动它。
pub const UMI_OFFLINE: i32 = 3;
/// 其余识别失败（没装微信、超时、响应形状不对……）。
pub const FAILED: i32 = 4;

/// 识别结果在界面上的编号，`0` = 出字了。`service` 是 [`current`] 给的服务名。
fn failure_code(result: &Result<String, Error>, service: Option<&str>) -> i32 {
    match result {
        Ok(text) if !text.trim().is_empty() => 0,
        Ok(_) => NO_TEXT,
        Err(Error::NotConfigured("recognize_services")) => NO_SERVICE,
        Err(Error::Http {
            kind: HttpKind::Connect,
            ..
        }) if service == Some("umi") => UMI_OFFLINE,
        Err(_) => FAILED,
    }
}

/// 测试连接（设置界面按「测试连接」时调）。后台线程调，最长 30 秒（R-5）。
pub fn test_umi(config: &umi::Config) -> Result<(), Error> {
    service::umi::test(config).inspect_err(|e| log::warn!("Recognize: umi test failed: {e}"))
}

/// 微信 OCR 的「测试连接」：探路径，成功给微信版本号（旧版弹窗里的「微信版本」一行）。
/// 探的是本机装没装、插件下没下，不真识别，所以很快，但还是别在 UI 线程上调（R-5）。
pub fn test_wechat() -> Result<String, Error> {
    platform::wechat_ocr_status().inspect_err(|e| log::warn!("Recognize: wechat test failed: {e}"))
}

/// Apple Vision 的「测试连接」：探 sidecar 在不在，成功给引擎名。很快，但还是别在 UI 线程上调（R-5）。
pub fn test_apple() -> Result<String, Error> {
    platform::apple_ocr_status().inspect_err(|e| log::warn!("Recognize: apple test failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::config::Instance;
    use serde_json::json;

    #[test]
    fn first_enabled_picks_apple_when_enabled() {
        let apple = Service::Apple(Instance::new("apple"));
        let umi = Service::Umi(Instance::new("umi"));
        let services = vec![apple.clone(), umi];
        assert_eq!(first_enabled(&services), Some(&apple));
        assert_eq!(service_name(&apple), Some("apple"));
    }

    #[test]
    fn first_enabled_picks_first_when_enabled() {
        let wechat = Service::Wechat(Instance::new("wechat"));
        let umi = Service::Umi(Instance::new("umi"));
        let services = vec![wechat.clone(), umi];
        assert_eq!(first_enabled(&services), Some(&wechat));
    }

    #[test]
    fn first_enabled_disabled_picks_second() {
        let mut wechat_inst = Instance::new("wechat");
        wechat_inst.enabled = false;
        let wechat = Service::Wechat(wechat_inst);

        let umi = Service::Umi(Instance::new("umi"));
        let services = vec![wechat, umi.clone()];
        assert_eq!(first_enabled(&services), Some(&umi));
    }

    #[test]
    fn first_enabled_all_disabled_returns_none() {
        let mut wechat_inst = Instance::new("wechat");
        wechat_inst.enabled = false;
        let mut umi_inst = Instance::new("umi");
        umi_inst.enabled = false;

        let services = vec![Service::Wechat(wechat_inst), Service::Umi(umi_inst)];
        assert_eq!(first_enabled(&services), None);
    }

    #[test]
    fn first_enabled_empty_list_returns_none() {
        let services: Vec<Service> = Vec::new();
        assert_eq!(first_enabled(&services), None);
    }

    #[test]
    fn first_enabled_unknown_is_skipped() {
        let unknown: Service = serde_json::from_value(json!({
            "id": "future",
            "kind": "future_recognize",
            "enabled": true
        }))
        .unwrap();
        let mut wechat_inst = Instance::new("wechat");
        wechat_inst.enabled = false;

        let services = vec![unknown, Service::Wechat(wechat_inst)];
        assert_eq!(first_enabled(&services), None);
    }

    #[test]
    fn first_enabled_unknown_in_front_picks_following_real_service() {
        let unknown: Service = serde_json::from_value(json!({
            "id": "future",
            "kind": "future_recognize",
            "enabled": true
        }))
        .unwrap();
        let umi = Service::Umi(Instance::new("umi"));

        let services = vec![unknown, umi.clone()];
        assert_eq!(first_enabled(&services), Some(&umi));
    }

    #[test]
    fn failure_codes_cover_every_empty_result() {
        assert_eq!(failure_code(&Ok("hello".into()), Some("wechat")), 0);
        // 只有空白也算没出字
        assert_eq!(failure_code(&Ok("  \n ".into()), Some("wechat")), NO_TEXT);
        assert_eq!(failure_code(&Ok(String::new()), Some("umi")), NO_TEXT);
        assert_eq!(
            failure_code(&Err(Error::NotConfigured("recognize_services")), None),
            NO_SERVICE
        );
        let offline = || Error::Http {
            status: None,
            kind: HttpKind::Connect,
        };
        assert_eq!(failure_code(&Err(offline()), Some("umi")), UMI_OFFLINE);
        // 同样连不上，但用的不是 Umi：不能说「Umi-OCR 未运行」
        assert_eq!(failure_code(&Err(offline()), Some("wechat")), FAILED);
        assert_eq!(
            failure_code(
                &Err(Error::Platform("WeChat is not installed".into())),
                Some("wechat")
            ),
            FAILED
        );
        assert_eq!(failure_code(&Err(Error::Timeout), Some("umi")), FAILED);
    }

    #[test]
    fn first_enabled_skips_translation_services() {
        let google = Service::Google(Instance::new("google"));
        let umi = Service::Umi(Instance::new("umi"));

        let services = vec![google, umi.clone()];
        assert_eq!(first_enabled(&services), Some(&umi));
    }
}
