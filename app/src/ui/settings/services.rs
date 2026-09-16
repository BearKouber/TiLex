//! 设置窗口：服务设置页（列表、启用开关、删除、拖动排序）。
//! 不在 model 里保留 `Service::Unknown`，但保持真实下标以原样保留它们在 `config.json` 里。

use slint::{ComponentHandle, VecModel};
use std::rc::Rc;

use crate::logic::config::{self, Service};
use crate::logic::service_icon as icons;
use crate::slint_ui::{ServiceRow, SettingsWindow};

pub fn bind(page: &SettingsWindow) {
    refresh(page);

    let weak = page.as_weak();
    page.on_set_service_enabled(move |kind, idx, enabled| {
        if let Some(page) = weak.upgrade() {
            handle_set_service_enabled(&page, kind.as_str(), idx, enabled);
        }
    });

    let weak = page.as_weak();
    page.on_remove_service(move |kind, idx| {
        if let Some(page) = weak.upgrade() {
            handle_remove_service(&page, kind.as_str(), idx);
        }
    });

    let weak = page.as_weak();
    page.on_move_service(move |kind, from, to| {
        if let Some(page) = weak.upgrade() {
            handle_move_service(&page, kind.as_str(), from, to);
        }
    });

    let weak = page.as_weak();
    page.on_edit_service(move |kind, idx| {
        if let Some(page) = weak.upgrade() {
            handle_edit_service(&page, kind.as_str(), idx);
        }
    });

    page.on_add_ai_service(|kind| {
        log::info!("Settings: add AI service clicked for {}", kind.as_str());
    });

    let weak = page.as_weak();
    page.on_save_service(
        move |kind, idx, service, label, mode, custom_url, api_key, custom_api_url| {
            if let Some(page) = weak.upgrade() {
                handle_save_service(
                    &page,
                    ServiceDraft {
                        kind: kind.as_str(),
                        real_idx: idx,
                        service: service.as_str(),
                        label: label.as_str(),
                        mode: mode.as_str(),
                        custom_url: custom_url.as_str(),
                        api_key: api_key.as_str(),
                        custom_api_url: custom_api_url.as_str(),
                    },
                );
            }
        },
    );
}

pub fn refresh(page: &SettingsWindow) {
    let cfg = config::snapshot();
    let tr_rows = to_rows(&cfg.translate_services);
    let rec_rows = to_rows(&cfg.recognize_services);

    page.set_translate_services(Rc::new(VecModel::from(tr_rows)).into());
    page.set_recognize_services(Rc::new(VecModel::from(rec_rows)).into());

    let wechat_info = icons::get_icon("wechat");
    page.set_wechat_icon_color(parse_hex_color(wechat_info.color));
}

fn to_rows(services: &[Service]) -> Vec<ServiceRow> {
    services
        .iter()
        .enumerate()
        .filter_map(|(real_idx, service)| {
            let (kind, icon_id, label, enabled) = match service {
                Service::Google(i) => ("google", "google", &i.label, i.enabled),
                Service::Wechat(i) => ("wechat", "wechat", &i.label, i.enabled),
                // 旧实例存过 icon 就照它显示，否则按地址 / 模型名猜厂商
                Service::Ai(i) => (
                    "ai",
                    i.extra
                        .get("icon")
                        .and_then(|v| v.as_str())
                        .unwrap_or_else(|| icons::match_icon(&i.config.base_url, &i.config.model)),
                    &i.label,
                    i.enabled,
                ),
                // 旧版 whetherAvailableService 行为：界面不显示认不出的服务
                Service::Unknown(_) => return None,
            };
            let info = icons::get_icon(icon_id);
            Some(ServiceRow {
                index: real_idx as i32,
                id: info.id.into(),
                kind: kind.into(),
                label: label.clone().into(),
                enabled,
                has_logo: info.has_file,
                icon_color: parse_hex_color(info.color),
                icon_letter: info.letter.into(),
            })
        })
        .collect()
}

/// `#RRGGBB` → 不透明 Brush。取值只来自 `service_icon` 里的编译期常量，认不出就给黑色。
fn parse_hex_color(hex: &str) -> slint::Brush {
    let rgb = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0);
    slint::Color::from_argb_encoded(0xFF00_0000 | rgb).into()
}

/// 成功时**不**重建 model：`for` 里的 Switch 会跟着整行一起重新实例化，滑块动画就丢了。
/// Switch 自己已经翻到新状态，配置也落了盘，两边一致；只有写失败才 `refresh` 把开关拨回去。
fn handle_set_service_enabled(page: &SettingsWindow, kind: &str, real_idx: i32, enabled: bool) {
    let res = config::update(|c| {
        let list = match kind {
            "translate" => &mut c.translate_services,
            "recognize" => &mut c.recognize_services,
            _ => return,
        };
        if let Some(service) = list.get_mut(real_idx as usize) {
            match service {
                Service::Google(inst) => inst.enabled = enabled,
                Service::Ai(inst) => inst.enabled = enabled,
                Service::Wechat(inst) => inst.enabled = enabled,
                Service::Unknown(_) => {}
            }
        }
    });
    if let Err(e) = res {
        log::warn!("Settings: save service enabled failed: {e}");
        page.invoke_show_save_error(e.to_string().into());
        refresh(page);
    }
}

fn handle_remove_service(page: &SettingsWindow, kind: &str, real_idx: i32) {
    let cfg = config::snapshot();
    if kind == "translate" {
        let visible_count = cfg
            .translate_services
            .iter()
            .filter(|s| !matches!(s, Service::Unknown(_)))
            .count();
        if visible_count <= 1 {
            page.invoke_show_least_services_warning();
            return;
        }
    }

    let res = config::update(|c| {
        let list = match kind {
            "translate" => &mut c.translate_services,
            "recognize" => &mut c.recognize_services,
            _ => return,
        };
        let idx = real_idx as usize;
        if idx < list.len() {
            list.remove(idx);
        }
    });
    if let Err(e) = res {
        log::warn!("Settings: remove service failed: {e}");
        page.invoke_show_save_error(e.to_string().into());
    }
    refresh(page);
}

fn handle_move_service(page: &SettingsWindow, kind: &str, from_real_idx: i32, to_real_idx: i32) {
    if from_real_idx == to_real_idx {
        return;
    }
    let res = config::update(|c| {
        let list = match kind {
            "translate" => &mut c.translate_services,
            "recognize" => &mut c.recognize_services,
            _ => return,
        };
        let from = from_real_idx as usize;
        let to = to_real_idx as usize;
        if from < list.len() && to < list.len() {
            let item = list.remove(from);
            list.insert(to, item);
        }
    });
    if let Err(e) = res {
        log::warn!("Settings: move service failed: {e}");
        page.invoke_show_save_error(e.to_string().into());
    }
    refresh(page);
}

fn handle_edit_service(page: &SettingsWindow, kind: &str, real_idx: i32) {
    let cfg = config::snapshot();
    let list = match kind {
        "translate" => &cfg.translate_services,
        "recognize" => &cfg.recognize_services,
        _ => return,
    };
    if let Some(service) = list.get(real_idx as usize) {
        match service {
            Service::Google(inst) => {
                page.set_draft_label(inst.label.as_str().into());
                // 界面上存的是下标（见 service_dialogs.slint 的注记）：0 web / 1 custom_api / 2 api
                page.set_draft_google_mode_index(match inst.config.mode.as_str() {
                    "custom_api" => 1,
                    "api" => 2,
                    _ => 0,
                });
                page.set_draft_google_custom_url(inst.config.custom_url.as_str().into());
                page.set_draft_google_api_key(inst.config.api_key.as_str().into());
                page.set_draft_google_custom_api_url(inst.config.custom_api_url.as_str().into());
                page.set_dialog_kind(kind.into());
                page.set_dialog_service("google".into());
                page.set_dialog_index(real_idx);
                page.set_dialog(3);
            }
            Service::Wechat(inst) => {
                page.set_draft_label(inst.label.as_str().into());
                page.set_dialog_kind(kind.into());
                page.set_dialog_service("wechat".into());
                page.set_dialog_index(real_idx);
                page.set_dialog(3);
            }
            _ => {
                log::info!("Settings: editing service kind not supported in this batch");
            }
        }
    }
}

struct ServiceDraft<'a> {
    kind: &'a str,
    real_idx: i32,
    service: &'a str,
    label: &'a str,
    mode: &'a str,
    custom_url: &'a str,
    api_key: &'a str,
    custom_api_url: &'a str,
}

fn handle_save_service(page: &SettingsWindow, draft: ServiceDraft<'_>) {
    let res = config::update(|c| {
        let list = match draft.kind {
            "translate" => &mut c.translate_services,
            "recognize" => &mut c.recognize_services,
            _ => return,
        };
        apply_draft(list, &draft);
    });

    match res {
        Ok(()) => {
            refresh(page);
            page.set_dialog(0);
        }
        Err(e) => {
            log::warn!("Settings: save service failed: {e}");
            page.invoke_show_save_error(e.to_string().into());
        }
    }
}

/// 把对话框的草稿写进服务列表：`real_idx == -1` 追加一个新实例，否则改这一项，
/// 保留它原有的 `enabled` 和 `extra`（不认识的字段不能因为一次保存就丢）。
fn apply_draft(list: &mut Vec<Service>, draft: &ServiceDraft<'_>) {
    if draft.real_idx == -1 {
        match draft.service {
            "google" => {
                let mut inst = config::Instance::<config::GoogleConfig>::new(
                    &config::new_instance_id("google"),
                );
                inst.label = draft.label.trim().to_string();
                inst.config.mode = draft.mode.to_string();
                inst.config.custom_url = draft.custom_url.trim().to_string();
                inst.config.api_key = draft.api_key.trim().to_string();
                inst.config.custom_api_url = draft.custom_api_url.trim().to_string();
                list.push(Service::Google(inst));
            }
            "wechat" => {
                let mut inst =
                    config::Instance::<config::NoSettings>::new(&config::new_instance_id("wechat"));
                inst.label = draft.label.trim().to_string();
                list.push(Service::Wechat(inst));
            }
            _ => {}
        }
    } else if let Some(item) = list.get_mut(draft.real_idx as usize) {
        match item {
            Service::Google(inst) => {
                inst.label = draft.label.trim().to_string();
                inst.config.mode = draft.mode.to_string();
                inst.config.custom_url = draft.custom_url.trim().to_string();
                inst.config.api_key = draft.api_key.trim().to_string();
                inst.config.custom_api_url = draft.custom_api_url.trim().to_string();
            }
            Service::Wechat(inst) => {
                inst.label = draft.label.trim().to_string();
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::config::Instance;
    use crate::service::ai;
    use serde_json::json;

    #[test]
    fn test_unknown_service_preservation_on_sort_and_delete() {
        let unknown_json = json!({
            "kind": "some_future_plugin",
            "api_key": "secret_key_12345",
            "nested": {
                "foo": "bar",
                "count": 42
            }
        });

        let mut services = vec![
            Service::Google(Instance::new("google")),
            Service::Unknown(unknown_json.clone()),
            Service::Ai(Instance {
                id: "ai@test1".into(),
                enabled: true,
                label: "AI First".into(),
                config: ai::Config {
                    base_url: "https://api.deepseek.com/v1".into(),
                    api_key: "key1".into(),
                    model: "deepseek-chat".into(),
                    ..Default::default()
                },
                extra: Default::default(),
            }),
            Service::Ai(Instance {
                id: "ai@test2".into(),
                enabled: false,
                label: "AI Second".into(),
                config: ai::Config {
                    base_url: "https://api.siliconflow.cn/v1".into(),
                    api_key: "key2".into(),
                    model: "deepseek-ai/DeepSeek-V3".into(),
                    ..Default::default()
                },
                extra: Default::default(),
            }),
        ];

        // 1. 验证 to_rows：Unknown 项在界面不显示，只有 3 个 row，且 index 记录的是真实下标
        let rows = to_rows(&services);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].index, 0); // google
        assert_eq!(rows[0].id.as_str(), "google");
        assert_eq!(rows[1].index, 2); // ai 1 (真实下标 2)
        assert_eq!(rows[1].id.as_str(), "deepseek");
        assert_eq!(rows[2].index, 3); // ai 2 (真实下标 3)
        assert_eq!(rows[2].id.as_str(), "siliconflow");

        // 2. 模拟拖动排序：把 real_idx 3 (AI Second) 移动到 real_idx 0 (Google 前面)
        let item = services.remove(3);
        services.insert(0, item);

        // 此时数组为：[AI Second, Google, Unknown, AI First]
        // 验证 Unknown 仍然在列表中，且内容完全一致
        assert_eq!(services[2], Service::Unknown(unknown_json.clone()));
        assert_eq!(services[0].id(), "ai@test2");
        assert_eq!(services[1].id(), "google");
        assert_eq!(services[3].id(), "ai@test1");

        // 3. 模拟删除：删除 real_idx 1 (Google)
        services.remove(1);
        // 此时数组为：[AI Second, Unknown, AI First]
        assert_eq!(services.len(), 3);
        assert_eq!(services[1], Service::Unknown(unknown_json.clone()));

        // 4. 再次生成 rows，检查真实下标重新对齐
        let rows_after = to_rows(&services);
        assert_eq!(rows_after.len(), 2);
        assert_eq!(rows_after[0].index, 0);
        assert_eq!(rows_after[1].index, 2);

        // 5. 序列化后再反序列化，Unknown 完整保留
        let json_str = serde_json::to_string(&services).unwrap();
        let deserialized: Vec<Service> = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized[1], Service::Unknown(unknown_json));
    }

    trait ServiceExt {
        fn id(&self) -> &str;
    }

    impl ServiceExt for Service {
        fn id(&self) -> &str {
            match self {
                Service::Google(inst) => &inst.id,
                Service::Ai(inst) => &inst.id,
                Service::Wechat(inst) => &inst.id,
                Service::Unknown(_) => "unknown",
            }
        }
    }

    fn draft<'a>(
        real_idx: i32,
        service: &'a str,
        label: &'a str,
        mode: &'a str,
    ) -> ServiceDraft<'a> {
        ServiceDraft {
            kind: "translate",
            real_idx,
            service,
            label,
            mode,
            custom_url: "",
            api_key: "",
            custom_api_url: "",
        }
    }

    #[test]
    fn apply_draft_adds_independent_instances() {
        let mut list = Vec::new();
        let mut first = draft(-1, "google", "  A  ", "web");
        first.custom_url = "https://mirror1.example.com";
        apply_draft(&mut list, &first);
        let mut second = draft(-1, "google", "B", "custom_api");
        second.custom_api_url = "https://proxy.example.com";
        apply_draft(&mut list, &second);

        assert_eq!(list.len(), 2);
        assert_ne!(list[0].id(), list[1].id());
        let (Service::Google(a), Service::Google(b)) = (&list[0], &list[1]) else {
            panic!("expected two google services");
        };
        assert_eq!(a.label, "A", "实例名前后的空格应去掉");
        assert_eq!(a.config.custom_url, "https://mirror1.example.com");
        // 两个实例互不串
        assert_eq!(a.config.custom_api_url, "");
        assert_eq!(b.config.mode, "custom_api");
        assert_eq!(b.config.custom_api_url, "https://proxy.example.com");
    }

    #[test]
    fn apply_draft_edit_preserves_enabled_and_extra() {
        let unknown = serde_json::json!({"kind": "future_ai", "id": "future@1"});
        let mut inst = Instance::<config::GoogleConfig>::new("google@test");
        inst.enabled = false;
        inst.label = "Old Name".into();
        inst.extra
            .insert("my_future_field".into(), serde_json::json!(1));
        let mut list = vec![Service::Unknown(unknown.clone()), Service::Google(inst)];

        // 真实下标 1（前面夹着一个界面上看不见的 Unknown）
        let mut edit = draft(1, "google", "New Name", "custom_api");
        edit.custom_api_url = "https://proxy.example.com";
        apply_draft(&mut list, &edit);

        let Service::Google(edited) = &list[1] else {
            panic!("expected google service");
        };
        assert_eq!(edited.label, "New Name");
        assert_eq!(edited.config.mode, "custom_api");
        assert_eq!(edited.config.custom_api_url, "https://proxy.example.com");
        assert_eq!(edited.id, "google@test", "编辑不该换 id");
        assert!(!edited.enabled, "编辑不该动 enabled");
        assert_eq!(
            edited.extra.get("my_future_field"),
            Some(&serde_json::json!(1)),
            "编辑不该丢未知字段"
        );
        assert_eq!(list[0], Service::Unknown(unknown), "Unknown 一字不变");
    }
}
