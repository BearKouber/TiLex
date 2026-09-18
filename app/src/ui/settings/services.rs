//! 设置窗口：服务设置页（列表、启用开关、删除、拖动排序）。
//! 不在 model 里保留 `Service::Unknown`，但保持真实下标以原样保留它们在 `config.json` 里。

use slint::{ComponentHandle, Model, VecModel};
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::error::{Error, HttpKind};
use crate::logic::config::{self, AiConfig, GoogleConfig, Service, UmiConfig};
use crate::logic::service_icon as icons;
use crate::logic::translate;
use crate::logic::{ai_presets, benchmark, model_cache, recognize};
use crate::slint_ui::{ModelRow, ServiceRow, SettingsWindow};

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

    let weak = page.as_weak();
    page.on_refresh_resolved_url(move |base, model, protocol, locked_icon| {
        if let Some(page) = weak.upgrade() {
            let url =
                ai_presets::resolved_chat_url(base.as_str(), model.as_str(), protocol.as_str());
            page.set_resolved_chat_url(url.into());

            let info = resolve_draft_icon(locked_icon.as_str(), base.as_str(), model.as_str());
            page.set_draft_ai_has_logo(info.has_file);
            page.set_draft_ai_icon_color(parse_hex_color(info.color));
            page.set_draft_ai_icon_letter(info.letter.into());

            refresh_models_panel(&page, base.as_str(), protocol.as_str(), model.as_str());
        }
    });

    let weak = page.as_weak();
    page.on_save_service(move |draft| {
        if let Some(page) = weak.upgrade() {
            handle_save_service(&page, ServiceDraft::from(&draft));
        }
    });

    let weak = page.as_weak();
    page.on_test_service(move |draft| {
        if let Some(page) = weak.upgrade() {
            handle_test_service(&page, ServiceDraft::from(&draft));
        }
    });

    let weak = page.as_weak();
    page.on_fetch_models(move |draft| {
        if let Some(page) = weak.upgrade() {
            handle_fetch_models(&page, ServiceDraft::from(&draft));
        }
    });

    let weak = page.as_weak();
    benchmark::set_notifier(Box::new(move || {
        let weak = weak.clone();
        // ignore: 窗口可能已经关了，测速照样跑完
        let _ = slint::invoke_from_event_loop(move || {
            if let Some(page) = weak.upgrade() {
                refresh_models_panel_from_page(&page);
            }
        });
    }));

    let weak = page.as_weak();
    page.on_start_benchmark(move |draft, force| {
        if let Some(page) = weak.upgrade() {
            handle_start_benchmark(&page, ServiceDraft::from(&draft), force);
        }
    });

    let weak = page.as_weak();
    page.on_stop_benchmark(move |draft| {
        if let Some(page) = weak.upgrade() {
            handle_stop_benchmark(&page, ServiceDraft::from(&draft));
        }
    });
}

pub fn refresh(page: &SettingsWindow) {
    let cfg = config::snapshot();
    let tr_rows = to_rows(&cfg.translate_services);
    let rec_rows = to_rows(&cfg.recognize_services);

    page.set_translate_services(Rc::new(VecModel::from(tr_rows)).into());
    page.set_recognize_services(Rc::new(VecModel::from(rec_rows)).into());

    let wechat_info = icons::get_icon("wechat");
    page.set_wechat_icon_color(parse_hex_color(wechat_info.color));
    let umi_info = icons::get_icon("umi");
    page.set_umi_icon_color(parse_hex_color(umi_info.color));

    // 识别服务每种只能装一个，装过的就从「添加内置服务」里去掉（旧版 `Recognize/index.jsx` 的 addable）。
    // 翻译服务不受这条限制：同一家可以配多个实例（不同镜像、不同 key）。
    page.set_recognize_has_wechat(
        cfg.recognize_services
            .iter()
            .any(|s| matches!(s, Service::Wechat(_))),
    );
    page.set_recognize_has_umi(
        cfg.recognize_services
            .iter()
            .any(|s| matches!(s, Service::Umi(_))),
    );

    page.set_default_ai_custom_instructions(ai_presets::default_custom_instructions().into());

    let presets: Vec<crate::slint_ui::AiPreset> = ai_presets::AI_PRESETS
        .iter()
        .map(|p| {
            let info = icons::get_icon(p.id);
            crate::slint_ui::AiPreset {
                id: p.id.into(),
                name: p.name.into(),
                base_url: p.base_url.into(),
                protocol: p.protocol.into(),
                has_logo: info.has_file,
                icon_color: parse_hex_color(info.color),
                icon_letter: info.letter.into(),
            }
        })
        .collect();
    page.set_ai_presets(Rc::new(VecModel::from(presets)).into());
}

fn to_rows(services: &[Service]) -> Vec<ServiceRow> {
    services
        .iter()
        .enumerate()
        .filter_map(|(real_idx, service)| {
            let (kind, icon_id, label, enabled) = match service {
                Service::Google(i) => ("google", "google", &i.label, i.enabled),
                Service::Wechat(i) => ("wechat", "wechat", &i.label, i.enabled),
                Service::Umi(i) => ("umi", "umi", &i.label, i.enabled),
                // B5 01 / 02 轮先让服务本身能用；品牌图标和配置表单在 03 轮补
                Service::Bing(i) => ("bing", icons::FALLBACK_ICON, &i.label, i.enabled),
                Service::Deepl(i) => ("deepl", icons::FALLBACK_ICON, &i.label, i.enabled),
                Service::Baidu(i) => ("baidu", icons::FALLBACK_ICON, &i.label, i.enabled),
                Service::Transmart(i) => ("transmart", icons::FALLBACK_ICON, &i.label, i.enabled),
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

/// AI 配置表单里该显示哪张脸：从预设进来的锁死（`locked` = 预设 id，改地址也不换），
/// 「自定义」进来的（`locked` 为空）跟着地址和模型走，认不出回落 sparkle。
fn resolve_draft_icon(locked: &str, base: &str, model: &str) -> &'static icons::IconInfo {
    let id = if locked.is_empty() {
        icons::match_icon(base, model)
    } else {
        locked
    };
    icons::get_icon(id)
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
                Service::Umi(inst) => inst.enabled = enabled,
                Service::Bing(inst) => inst.enabled = enabled,
                Service::Deepl(inst) => inst.enabled = enabled,
                Service::Baidu(inst) => inst.enabled = enabled,
                Service::Transmart(inst) => inst.enabled = enabled,
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
            Service::Umi(inst) => {
                page.set_draft_label(inst.label.as_str().into());
                page.set_draft_umi_url(inst.config.url.as_str().into());
                page.set_dialog_kind(kind.into());
                page.set_dialog_service("umi".into());
                page.set_dialog_index(real_idx);
                page.set_dialog(3);
            }
            Service::Ai(inst) => {
                page.set_draft_label(inst.label.as_str().into());
                page.set_draft_ai_base_url(inst.config.base_url.as_str().into());
                page.set_draft_ai_api_key(inst.config.api_key.as_str().into());
                page.set_draft_ai_model(inst.config.model.as_str().into());
                page.set_draft_ai_protocol_index(protocol_to_index(&inst.config.protocol));
                page.set_draft_ai_custom_instructions(
                    inst.config.custom_instructions.as_str().into(),
                );
                let icon_id = inst
                    .extra
                    .get("icon")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                page.set_draft_ai_icon(icon_id.into());
                let display_icon_id = if !icon_id.is_empty() {
                    icon_id
                } else {
                    icons::match_icon(&inst.config.base_url, &inst.config.model)
                };
                let info = icons::get_icon(display_icon_id);
                page.set_draft_ai_has_logo(info.has_file);
                page.set_draft_ai_icon_color(parse_hex_color(info.color));
                page.set_draft_ai_icon_letter(info.letter.into());
                let resolved = ai_presets::resolved_chat_url(
                    &inst.config.base_url,
                    &inst.config.model,
                    &inst.config.protocol,
                );
                page.set_resolved_chat_url(resolved.into());
                refresh_models_panel(
                    page,
                    &inst.config.base_url,
                    &inst.config.protocol,
                    &inst.config.model,
                );
                page.set_dialog_kind(kind.into());
                page.set_dialog_service("ai".into());
                page.set_dialog_index(real_idx);
                page.set_dialog(3);
            }
            Service::Bing(_)
            | Service::Deepl(_)
            | Service::Baidu(_)
            | Service::Transmart(_)
            | Service::Unknown(_) => {
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
    ai_base_url: &'a str,
    ai_api_key: &'a str,
    ai_model: &'a str,
    ai_protocol: &'a str,
    ai_custom_instructions: &'a str,
    ai_icon: &'a str,
    umi_url: &'a str,
}

impl<'a> From<&'a crate::slint_ui::ServiceDraft> for ServiceDraft<'a> {
    fn from(d: &'a crate::slint_ui::ServiceDraft) -> Self {
        Self {
            kind: d.kind.as_str(),
            real_idx: d.real_index,
            service: d.service.as_str(),
            label: d.label.as_str(),
            mode: d.google_mode.as_str(),
            custom_url: d.google_custom_url.as_str(),
            api_key: d.google_api_key.as_str(),
            custom_api_url: d.google_custom_api_url.as_str(),
            ai_base_url: d.ai_base_url.as_str(),
            ai_api_key: d.ai_api_key.as_str(),
            ai_model: d.ai_model.as_str(),
            ai_protocol: d.ai_protocol.as_str(),
            ai_custom_instructions: d.ai_custom_instructions.as_str(),
            ai_icon: d.ai_icon.as_str(),
            umi_url: d.umi_url.as_str(),
        }
    }
}

fn draft_google_config(draft: &ServiceDraft<'_>) -> GoogleConfig {
    GoogleConfig {
        mode: draft.mode.to_string(),
        custom_url: draft.custom_url.trim().to_string(),
        api_key: draft.api_key.trim().to_string(),
        custom_api_url: draft.custom_api_url.trim().to_string(),
    }
}

/// `base`：编辑已有实例时传它现在的配置，界面上编不到的字段（`request_arguments`、
/// `legacy_reference_instructions`）原样保留；新建传 `None` 用 `AiConfig::default()`。
fn draft_ai_config(draft: &ServiceDraft<'_>, base: Option<&AiConfig>) -> AiConfig {
    let mut config = base.cloned().unwrap_or_default();
    config.base_url = draft.ai_base_url.trim().to_string();
    config.api_key = draft.ai_api_key.trim().to_string();
    config.model = draft.ai_model.trim().to_string();
    config.protocol = draft.ai_protocol.trim().to_string();
    config.custom_instructions = draft.ai_custom_instructions.trim().to_string();
    config
}

/// 地址留空就用默认的本地地址（`umi::Config::default()`），和旧版 `config.url || defaultConfig.url` 一致。
fn draft_umi_config(draft: &ServiceDraft<'_>) -> UmiConfig {
    let url = draft.umi_url.trim();
    if url.is_empty() {
        UmiConfig::default()
    } else {
        UmiConfig {
            url: url.to_string(),
        }
    }
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        let mut truncated: String = s.chars().take(max_chars).collect();
        truncated.push('…');
        truncated
    } else {
        s.to_string()
    }
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

enum TestTarget {
    Google(GoogleConfig),
    Ai(AiConfig),
    Umi(UmiConfig),
    Wechat,
}

/// 测试通了之后结果条上写什么：翻译服务回显译文，微信回显探到的版本，其余只报通没通。
enum TestOk {
    Translated(String),
    WechatVersion(String),
    Reached,
}

fn handle_test_service(page: &SettingsWindow, draft: ServiceDraft<'_>) {
    if page.get_testing_service() {
        return;
    }
    page.set_testing_service(true);

    let target = match draft.service {
        "google" => TestTarget::Google(draft_google_config(&draft)),
        "ai" => {
            // 编辑已有实例时，界面上编不到的字段（request_arguments 等）从它现在的配置里接着用。
            let base = config::snapshot()
                .translate_services
                .get(usize::try_from(draft.real_idx).unwrap_or(usize::MAX))
                .and_then(|s| match s {
                    Service::Ai(inst) => Some(inst.config.clone()),
                    _ => None,
                });
            TestTarget::Ai(draft_ai_config(&draft, base.as_ref()))
        }
        "umi" => TestTarget::Umi(draft_umi_config(&draft)),
        "wechat" => TestTarget::Wechat,
        _ => {
            log::warn!(
                "Settings: unexpected service kind for test: {}",
                draft.service
            );
            page.set_testing_service(false);
            return;
        }
    };

    let weak = page.as_weak();
    // 识别服务没有译文可回显，结果条走另一句；连不上 Umi 又是单独一句（多半是没启动）。
    let is_umi = matches!(target, TestTarget::Umi(_));
    if let Err(e) = std::thread::Builder::new()
        .name("service-test".into())
        .spawn(move || {
            let start = std::time::Instant::now();
            let res = match &target {
                TestTarget::Google(c) => translate::test_google(c).map(TestOk::Translated),
                TestTarget::Ai(c) => translate::test_ai(c).map(TestOk::Translated),
                TestTarget::Umi(c) => recognize::test_umi(c).map(|()| TestOk::Reached),
                TestTarget::Wechat => recognize::test_wechat().map(TestOk::WechatVersion),
            };
            let elapsed_ms = start.elapsed().as_millis().min(i32::MAX as u128) as i32;
            // ignore: window might be closed during async test
            let _ = slint::invoke_from_event_loop(move || {
                let Some(page) = weak.upgrade() else { return };
                page.set_testing_service(false);
                match res {
                    Ok(TestOk::Translated(text)) => {
                        let truncated = truncate_chars(&text, 80);
                        page.invoke_show_test_success(truncated.into(), elapsed_ms);
                    }
                    Ok(TestOk::WechatVersion(version)) => {
                        page.invoke_show_wechat_test_success(version.as_str().into(), elapsed_ms);
                    }
                    Ok(TestOk::Reached) => page.invoke_show_recognize_test_success(elapsed_ms),
                    Err(Error::Http {
                        kind: HttpKind::Connect,
                        ..
                    }) if is_umi => page.invoke_show_umi_not_running(elapsed_ms),
                    Err(e) => {
                        page.invoke_show_test_failed(e.to_string().into(), elapsed_ms);
                    }
                }
            });
        })
    {
        log::error!("Settings: spawn service-test thread failed: {e}");
        page.set_testing_service(false);
        page.invoke_show_test_failed(e.to_string().into(), 0);
    }
}

fn protocol_from_index(i: i32) -> &'static str {
    match i {
        1 => "openai_responses",
        2 => "anthropic",
        3 => "google",
        _ => "openai_chat",
    }
}

fn protocol_to_index(protocol: &str) -> i32 {
    match protocol {
        "openai_responses" => 1,
        "anthropic" => 2,
        "google" => 3,
        _ => 0,
    }
}

/// 面板的一行。排序和排名都在 Rust 算好，`.slint` 只管画。
/// state: "ms" | "testing" | "untested" | "failed"
fn model_rows(
    listed: &[String],
    latencies: &BTreeMap<String, Option<u32>>,
    testing: &[String],
) -> Vec<ModelRow> {
    struct TempRow {
        name: String,
        state: &'static str,
        ms: i32,
        category: u8,
    }

    let mut rows: Vec<TempRow> = listed
        .iter()
        .map(|m| {
            let name = m.clone();
            let is_testing = testing.iter().any(|t| t == m);
            if is_testing {
                TempRow {
                    name,
                    state: "testing",
                    ms: 0,
                    category: 1,
                }
            } else {
                match latencies.get(m) {
                    Some(Some(ms)) => TempRow {
                        name,
                        state: "ms",
                        ms: *ms as i32,
                        category: 0,
                    },
                    Some(None) => TempRow {
                        name,
                        state: "failed",
                        ms: 0,
                        category: 3,
                    },
                    None => TempRow {
                        name,
                        state: "untested",
                        ms: 0,
                        category: 2,
                    },
                }
            }
        })
        .collect();

    // 排序（旧版 ModelBenchmark.jsx:32-50）：
    // 1. 有数值的排最前，按毫秒升序；
    // 2. 正在测的（progress.current == name）排在有数值的后面；
    // 3. 没测过的；
    // 4. 失败的排最底下。
    rows.sort_by(|a, b| {
        if a.category == 0 && b.category == 0 {
            a.ms.cmp(&b.ms)
        } else {
            a.category.cmp(&b.category)
        }
    });

    rows.into_iter()
        .enumerate()
        .map(|(i, r)| {
            let rank = if r.state == "ms" { (i + 1) as i32 } else { 0 };
            ModelRow {
                name: r.name.into(),
                state: r.state.into(),
                ms: r.ms,
                rank,
            }
        })
        .collect()
}

/// 重算面板的行 + 进度 + 汇总，四个入口共用。
fn refresh_models_panel(page: &SettingsWindow, base: &str, protocol: &str, model: &str) {
    let endpoint = model_cache::endpoint_of(base, protocol);
    let cached = if endpoint.is_empty() {
        Vec::new()
    } else {
        model_cache::models(&endpoint)
    };
    let current = model.trim();
    let mut listed = Vec::new();
    if !current.is_empty() && !cached.iter().any(|m| m == current) {
        listed.push(current.to_owned());
    }
    listed.extend(cached);

    let latencies = if endpoint.is_empty() {
        BTreeMap::new()
    } else {
        model_cache::latencies(&endpoint)
    };
    let progress = if endpoint.is_empty() {
        benchmark::Progress::default()
    } else {
        benchmark::progress(&endpoint)
    };

    let rows = model_rows(&listed, &latencies, &progress.testing);
    let tested_count = rows.iter().filter(|r| r.state.as_str() == "ms").count() as i32;
    let failed_count = rows.iter().filter(|r| r.state.as_str() == "failed").count() as i32;

    page.set_ai_models(Rc::new(VecModel::from(rows)).into());
    page.set_bench_running(progress.running);
    page.set_bench_current(progress.current.into());
    page.set_bench_remaining(progress.remaining as i32);
    page.set_bench_tested(tested_count);
    page.set_bench_failed(failed_count);
}

/// 从窗口属性上读 base / protocol / model 再调上面那个（给通知回调用）。
fn refresh_models_panel_from_page(page: &SettingsWindow) {
    let base = page.get_draft_ai_base_url();
    let protocol = protocol_from_index(page.get_draft_ai_protocol_index());
    let model = page.get_draft_ai_model();
    refresh_models_panel(page, base.as_str(), protocol, model.as_str());
}

fn handle_fetch_models(page: &SettingsWindow, draft: ServiceDraft<'_>) {
    if page.get_fetching_models() {
        return;
    }
    page.set_fetching_models(true);

    let base = config::snapshot()
        .translate_services
        .get(usize::try_from(draft.real_idx).unwrap_or(usize::MAX))
        .and_then(|s| match s {
            Service::Ai(inst) => Some(inst.config.clone()),
            _ => None,
        });
    let ai_config = draft_ai_config(&draft, base.as_ref());

    let weak = page.as_weak();
    if let Err(e) = std::thread::Builder::new()
        .name("fetch-models".into())
        .spawn(move || {
            let res = model_cache::fetch(&ai_config);
            // ignore: window might be closed during async fetch
            let _ = slint::invoke_from_event_loop(move || {
                let Some(page) = weak.upgrade() else { return };
                page.set_fetching_models(false);
                match res {
                    Ok(list) => {
                        let count = list.len() as i32;
                        refresh_models_panel_from_page(&page);
                        page.invoke_show_fetch_success(count);
                    }
                    Err(e) => {
                        page.invoke_show_fetch_failed(e.to_string().into());
                    }
                }
            });
        })
    {
        log::error!("Settings: spawn fetch-models thread failed: {e}");
        page.set_fetching_models(false);
        page.invoke_show_fetch_failed(e.to_string().into());
    }
}

fn handle_start_benchmark(page: &SettingsWindow, draft: ServiceDraft<'_>, force: bool) {
    let base = config::snapshot()
        .translate_services
        .get(usize::try_from(draft.real_idx).unwrap_or(usize::MAX))
        .and_then(|s| match s {
            Service::Ai(inst) => Some(inst.config.clone()),
            _ => None,
        });
    let ai_config = draft_ai_config(&draft, base.as_ref());
    let endpoint = model_cache::endpoint_of(&ai_config.base_url, &ai_config.protocol);
    if endpoint.is_empty() {
        page.invoke_show_bench_error("base_url is required".into());
        return;
    }
    let models_model = page.get_ai_models();
    let models: Vec<String> = (0..models_model.row_count())
        .filter_map(|i| models_model.row_data(i).map(|r| r.name.to_string()))
        .collect();
    benchmark::start(&ai_config, models, force);
    refresh_models_panel_from_page(page);
}

fn handle_stop_benchmark(page: &SettingsWindow, draft: ServiceDraft<'_>) {
    let endpoint = model_cache::endpoint_of(draft.ai_base_url, draft.ai_protocol);
    benchmark::stop(&endpoint);
    refresh_models_panel_from_page(page);
}

/// 把对话框的草稿写进服务列表：`real_idx == -1` 追加一个新实例（默认关闭，由用户手动开启），
/// 否则改这一项，保留它原有的 `enabled` 和 `extra`（不认识的字段不能因为一次保存就丢）。
fn apply_draft(list: &mut Vec<Service>, draft: &ServiceDraft<'_>) {
    if draft.real_idx == -1 {
        match draft.service {
            "google" => {
                let mut inst =
                    config::Instance::<GoogleConfig>::new(&config::new_instance_id("google"));
                inst.enabled = false;
                inst.label = draft.label.trim().to_string();
                inst.config = draft_google_config(draft);
                list.push(Service::Google(inst));
            }
            "wechat" => {
                let mut inst =
                    config::Instance::<config::NoSettings>::new(&config::new_instance_id("wechat"));
                inst.enabled = false;
                inst.label = draft.label.trim().to_string();
                list.push(Service::Wechat(inst));
            }
            "umi" => {
                let mut inst = config::Instance::<UmiConfig>::new(&config::new_instance_id("umi"));
                inst.enabled = false;
                inst.label = draft.label.trim().to_string();
                inst.config = draft_umi_config(draft);
                list.push(Service::Umi(inst));
            }
            "ai" => {
                let mut inst = config::Instance::<AiConfig>::new(&config::new_instance_id("ai"));
                inst.enabled = false;
                inst.label = draft.label.trim().to_string();
                inst.config = draft_ai_config(draft, None);
                let icon = draft.ai_icon.trim();
                if !icon.is_empty() {
                    inst.extra.insert(
                        "icon".to_string(),
                        serde_json::Value::String(icon.to_string()),
                    );
                }
                list.push(Service::Ai(inst));
            }
            _ => {}
        }
    } else if let Some(item) = list.get_mut(draft.real_idx as usize) {
        match item {
            Service::Google(inst) => {
                inst.label = draft.label.trim().to_string();
                inst.config = draft_google_config(draft);
            }
            Service::Wechat(inst) => {
                inst.label = draft.label.trim().to_string();
            }
            Service::Umi(inst) => {
                inst.label = draft.label.trim().to_string();
                inst.config = draft_umi_config(draft);
            }
            Service::Ai(inst) => {
                inst.label = draft.label.trim().to_string();
                inst.config = draft_ai_config(draft, Some(&inst.config));
                if !inst.extra.contains_key("icon") {
                    let icon = draft.ai_icon.trim();
                    if !icon.is_empty() {
                        inst.extra.insert(
                            "icon".to_string(),
                            serde_json::Value::String(icon.to_string()),
                        );
                    }
                }
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
                Service::Umi(inst) => &inst.id,
                Service::Bing(inst) => &inst.id,
                Service::Deepl(inst) => &inst.id,
                Service::Baidu(inst) => &inst.id,
                Service::Transmart(inst) => &inst.id,
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
            ai_base_url: "",
            ai_api_key: "",
            ai_model: "",
            ai_protocol: "openai_chat",
            ai_custom_instructions: "",
            ai_icon: "",
            umi_url: "",
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
        assert!(!a.enabled && !b.enabled, "新加的服务默认关闭，要用户手动开");
        assert_eq!(a.label, "A", "实例名前后的空格应去掉");
        assert_eq!(a.config.custom_url, "https://mirror1.example.com");
        // 两个实例互不串
        assert_eq!(a.config.custom_api_url, "");
        assert_eq!(b.config.mode, "custom_api");
        assert_eq!(b.config.custom_api_url, "https://proxy.example.com");
    }

    #[test]
    fn apply_draft_adds_ai_service() {
        let mut list = Vec::new();
        let mut first = draft(-1, "ai", "  My Zhipu  ", "");
        first.ai_base_url = "https://open.bigmodel.cn/api/paas/v4";
        first.ai_api_key = "secret_key";
        first.ai_model = "glm-4";
        first.ai_protocol = "openai_chat";
        first.ai_custom_instructions = "  translate directly  ";
        first.ai_icon = "zhipu";
        apply_draft(&mut list, &first);

        assert_eq!(list.len(), 1);
        let Service::Ai(inst) = &list[0] else {
            panic!("expected ai service");
        };
        assert!(!inst.enabled, "新加的 AI 服务默认关闭");
        assert!(inst.id.starts_with("ai@"), "id 必须由 new_instance_id 生成");
        assert_eq!(inst.label, "My Zhipu");
        assert_eq!(inst.config.base_url, "https://open.bigmodel.cn/api/paas/v4");
        assert_eq!(inst.config.api_key, "secret_key");
        assert_eq!(inst.config.model, "glm-4");
        assert_eq!(inst.config.protocol, "openai_chat");
        assert_eq!(inst.config.custom_instructions, "translate directly");
        assert_eq!(inst.extra.get("icon"), Some(&serde_json::json!("zhipu")));

        // 自定义 AI：ai_icon 为空时，extra 里不放 icon
        let mut custom = draft(-1, "ai", "Custom AI", "");
        custom.ai_base_url = "https://my-proxy.com/v1";
        apply_draft(&mut list, &custom);
        assert_eq!(list.len(), 2);
        let Service::Ai(inst2) = &list[1] else {
            panic!("expected ai service");
        };
        assert_eq!(inst2.extra.get("icon"), None);
    }

    #[test]
    fn apply_draft_umi_url_defaults_when_blank() {
        let mut list = Vec::new();
        // 地址留空：回落到默认的本地地址，不能存成空串（旧版 `config.url || defaultConfig.url`）
        apply_draft(&mut list, &draft(-1, "umi", "  Local OCR  ", ""));
        let Service::Umi(inst) = &list[0] else {
            panic!("expected umi service");
        };
        assert!(!inst.enabled, "新加的识别服务默认关闭");
        assert!(inst.id.starts_with("umi@"));
        assert_eq!(inst.label, "Local OCR");
        assert_eq!(
            inst.config.url,
            crate::logic::config::UmiConfig::default().url
        );

        // 填了地址就用它，两侧空白去掉
        let mut custom = draft(-1, "umi", "Remote", "");
        custom.umi_url = "  http://192.168.1.9:1224/api/ocr  ";
        apply_draft(&mut list, &custom);
        let Service::Umi(inst) = &list[1] else {
            panic!("expected umi service");
        };
        assert_eq!(inst.config.url, "http://192.168.1.9:1224/api/ocr");

        // 编辑已有实例：改地址，enabled 不动
        let mut edit = draft(0, "umi", "Local OCR", "");
        edit.umi_url = "http://127.0.0.1:2224/api/ocr";
        apply_draft(&mut list, &edit);
        let Service::Umi(inst) = &list[0] else {
            panic!("expected umi service");
        };
        assert_eq!(inst.config.url, "http://127.0.0.1:2224/api/ocr");
        assert!(!inst.enabled);
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

    #[test]
    fn apply_draft_edits_ai_service_preserves_enabled_and_extra() {
        let mut inst = Instance::<config::AiConfig>::new("ai@test");
        inst.enabled = true;
        inst.label = "Old AI".into();
        inst.config.base_url = "https://api.openai.com/v1".into();
        inst.config.api_key = "old_key".into();
        inst.config
            .request_arguments
            .insert("temperature".into(), serde_json::json!(0.7));
        inst.config
            .request_arguments
            .insert("custom_arg".into(), serde_json::json!("val"));
        inst.extra
            .insert("icon".into(), serde_json::json!("openai"));
        inst.extra
            .insert("custom_extra_field".into(), serde_json::json!(42));
        let mut list = vec![Service::Ai(inst)];

        let mut edit = draft(0, "ai", "New AI", "");
        edit.ai_base_url = "https://api.openai.com/v2";
        edit.ai_api_key = "new_key";
        edit.ai_model = "gpt-4o";
        edit.ai_protocol = "openai_chat";
        edit.ai_custom_instructions = "keep original tone";
        edit.ai_icon = "openai";
        apply_draft(&mut list, &edit);

        let Service::Ai(edited) = &list[0] else {
            panic!("expected ai service");
        };
        assert_eq!(edited.id, "ai@test", "编辑不能改 id");
        assert!(edited.enabled, "编辑不能改 enabled 状态");
        assert_eq!(edited.label, "New AI");
        assert_eq!(edited.config.base_url, "https://api.openai.com/v2");
        assert_eq!(edited.config.api_key, "new_key");
        assert_eq!(edited.config.model, "gpt-4o");
        assert_eq!(edited.config.custom_instructions, "keep original tone");
        assert_eq!(
            edited.config.request_arguments.get("temperature"),
            Some(&serde_json::json!(0.7)),
            "编辑不能重置 request_arguments"
        );
        assert_eq!(
            edited.config.request_arguments.get("custom_arg"),
            Some(&serde_json::json!("val")),
            "编辑不能丢自定义生成参数"
        );
        assert_eq!(edited.extra.get("icon"), Some(&serde_json::json!("openai")));
        assert_eq!(
            edited.extra.get("custom_extra_field"),
            Some(&serde_json::json!(42)),
            "未知 extra 字段必须保留"
        );
    }

    #[test]
    fn test_truncate_chars() {
        assert_eq!(truncate_chars("hello", 10), "hello");
        assert_eq!(truncate_chars("你好世界", 4), "你好世界");
        assert_eq!(truncate_chars("你好世界！", 4), "你好世界…");
        assert_eq!(truncate_chars("abcdefghij", 5), "abcde…");
    }

    #[test]
    fn test_draft_configs() {
        let mut d = draft(-1, "google", "Google", "custom_api");
        d.custom_api_url = "https://custom.com";
        let g_cfg = draft_google_config(&d);
        assert_eq!(g_cfg.mode, "custom_api");
        assert_eq!(g_cfg.custom_api_url, "https://custom.com");

        let mut d_ai = draft(-1, "ai", "AI", "");
        d_ai.ai_base_url = "https://ai.example.com";
        d_ai.ai_model = "gpt-4";
        let ai_cfg_new = draft_ai_config(&d_ai, None);
        assert_eq!(ai_cfg_new.base_url, "https://ai.example.com");
        assert_eq!(ai_cfg_new.model, "gpt-4");
        assert!(ai_cfg_new.request_arguments.contains_key("temperature"));

        let mut base = AiConfig::default();
        base.request_arguments
            .insert("temperature".into(), serde_json::json!(0.8));
        base.legacy_reference_instructions = "legacy".into();
        let ai_cfg_edit = draft_ai_config(&d_ai, Some(&base));
        assert_eq!(ai_cfg_edit.base_url, "https://ai.example.com");
        assert_eq!(
            ai_cfg_edit.request_arguments.get("temperature"),
            Some(&serde_json::json!(0.8))
        );
        assert_eq!(ai_cfg_edit.legacy_reference_instructions, "legacy");
    }

    #[test]
    fn test_resolve_ai_icon_locked_vs_custom() {
        // 自定义（locked 为空）：认不出的地址回落 sparkle ✦
        let info = resolve_draft_icon("", "https://my-custom-proxy.com", "custom-model");
        assert_eq!(info.id, "sparkle");
        assert!(!info.has_file);
        assert_eq!(info.letter, "✦");

        // 自定义：填 deepseek 地址就换成 DeepSeek 的脸
        assert_eq!(
            resolve_draft_icon("", "https://api.deepseek.com", "").id,
            "deepseek"
        );

        // 预设进来的：地址改成 deepseek 也仍然锁着智谱
        assert_eq!(
            resolve_draft_icon("zhipu", "https://api.deepseek.com", "").id,
            "zhipu"
        );
    }

    #[test]
    fn test_model_rows_ranking_and_order() {
        let listed = vec![
            "model-c".to_string(),
            "model-b".to_string(),
            "model-a".to_string(),
            "model-testing".to_string(),
            "model-untested".to_string(),
            "model-failed".to_string(),
        ];
        let mut latencies = BTreeMap::new();
        latencies.insert("model-c".to_string(), Some(800));
        latencies.insert("model-b".to_string(), Some(120));
        latencies.insert("model-a".to_string(), Some(450));
        latencies.insert("model-failed".to_string(), None);

        let rows = model_rows(&listed, &latencies, &["model-testing".to_string()]);
        assert_eq!(rows.len(), 6);

        // 1. 数值升序排最前：model-b (120ms, #1), model-a (450ms, #2), model-c (800ms, #3)
        assert_eq!(rows[0].name.as_str(), "model-b");
        assert_eq!(rows[0].state.as_str(), "ms");
        assert_eq!(rows[0].ms, 120);
        assert_eq!(rows[0].rank, 1);

        assert_eq!(rows[1].name.as_str(), "model-a");
        assert_eq!(rows[1].state.as_str(), "ms");
        assert_eq!(rows[1].ms, 450);
        assert_eq!(rows[1].rank, 2);

        assert_eq!(rows[2].name.as_str(), "model-c");
        assert_eq!(rows[2].state.as_str(), "ms");
        assert_eq!(rows[2].ms, 800);
        assert_eq!(rows[2].rank, 3);

        // 2. 测试中排在数值后，rank = 0
        assert_eq!(rows[3].name.as_str(), "model-testing");
        assert_eq!(rows[3].state.as_str(), "testing");
        assert_eq!(rows[3].ms, 0);
        assert_eq!(rows[3].rank, 0);

        // 3. 未测排在测试中后面
        assert_eq!(rows[4].name.as_str(), "model-untested");
        assert_eq!(rows[4].state.as_str(), "untested");
        assert_eq!(rows[4].ms, 0);
        assert_eq!(rows[4].rank, 0);

        // 4. 失败排在最底下
        assert_eq!(rows[5].name.as_str(), "model-failed");
        assert_eq!(rows[5].state.as_str(), "failed");
        assert_eq!(rows[5].ms, 0);
        assert_eq!(rows[5].rank, 0);
    }
}
