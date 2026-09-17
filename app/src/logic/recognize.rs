//! 识别服务调度（prd F4）。
//!
//! 只取识别列表里第一个启用的服务识别图片。后台线程调用，不阻塞 UI 线程（R-5）。

use std::path::Path;

use crate::error::Error;
use crate::logic::config::{self, Service};
use crate::platform;
use crate::service::{self, umi};

/// 从识别服务列表中挑出第一个启用的识别服务。
///
/// 认不出的服务（`Unknown`）以及误入识别列表的非识别服务（如翻译服务）均会被跳过。
fn first_enabled(services: &[Service]) -> Option<&Service> {
    services.iter().find(|s| match s {
        Service::Wechat(i) => i.enabled,
        Service::Umi(i) => i.enabled,
        _ => false,
    })
}

fn service_name(service: &Service) -> Option<&'static str> {
    match service {
        Service::Wechat(_) => Some("wechat"),
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
        Service::Umi(i) => service::umi::recognize(&i.config, image),
        _ => return Err(Error::NotConfigured("recognize_services")),
    };
    match &result {
        Ok(text) => log::info!("Recognize: {name} {} chars", text.chars().count()),
        Err(e) => log::warn!("Recognize: {name} failed: {e}"),
    }
    result
}

/// 测试连接（设置界面按「测试连接」时调）。后台线程调，最长 30 秒（R-5）。
pub fn test_umi(config: &umi::Config) -> Result<(), Error> {
    service::umi::test(config).inspect_err(|e| log::warn!("Recognize: umi test failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::config::Instance;
    use serde_json::json;

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
    fn first_enabled_skips_translation_services() {
        let google = Service::Google(Instance::new("google"));
        let umi = Service::Umi(Instance::new("umi"));

        let services = vec![google, umi.clone()];
        assert_eq!(first_enabled(&services), Some(&umi));
    }
}
