//! 0.1.2（Tauri 版）→ 0.2.x 的一次性数据迁移（B10 P17）。
//!
//! 两版把数据写在两个目录：旧版 `<appdata>/com.tilex.desktop/`、新版 `<appdata>/TiLex/`。
//! 新版从来不看旧目录，不迁的话用户升上来就是「设置和生词本全没了」（数据还在，只是没人读）。
//!
//! 三条红线：
//! - **只读旧目录**，绝不删、绝不改 —— 用户回滚到 0.1.2 还得能用。
//! - **迁移失败不挡启动**：这里的错误一律只记日志，调用方拿不到 `Result`。
//! - **认不出的服务走 `Service::Unknown` 原样保留**，宁可界面上不显示，也不能丢用户的 API key。
//!
//! 触发条件就是「新目录里那个文件不存在」，配置和生词本各自判断，不另写标记文件。
//! 代价是用户清空新目录后重装会再迁一次（取舍记在 prd P17）。

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::{Map, Value};

use crate::error::Error;
use crate::logic::config::{self, Config, Service};
use crate::logic::wordbook;
use crate::service::ai;

/// 旧版的目录名（Tauri 按 bundle identifier 定的）。和新目录同级。
const LEGACY_DIR: &str = "com.tilex.desktop";

/// 旧版 0.1.2 出厂的那两条 prompt（`src/services/translate/ai/instructions.js:16`）。
/// 用户没改过 prompt 时存的就是它们，折成新版的默认「自定义要求」而不是照抄成一段英文。
const LEGACY_DEFAULT_PROMPTS: [(&str, &str); 2] = [
    (
        "system",
        "You are a professional translation engine, please translate the text into a colloquial, professional, elegant and fluent content, without the style of machine translation. You must only translate the text content, never interpret it.",
    ),
    ("user", "Translate into $to:\n\"\"\"\n$text\n\"\"\""),
];

/// 旧 prompt 里 `$text` 这类占位符的说明，照抄 `instructions.js:40` 的原文。
/// 只有折出来的要求里真的出现了占位符才填，否则是白给模型加一段废话。
const LEGACY_REFERENCES: &str = "These customInstructions were migrated from legacy prompts: $text refers to the separately supplied source text, $from to sourceLanguage, $to to targetLanguage, and $detect to detectedLanguage. Interpret these references without rewriting the source text or changing the JSON output contract.";

/// 在 `config::init` 之前调用一次。旧目录不存在、或者新目录已经有数据，就什么都不做。
pub fn run(data: &Path) {
    let Some(old) = legacy_dir(data) else {
        return;
    };
    if !old.is_dir() {
        return;
    }
    log::info!("Migrate: legacy profile found at {}", old.display());

    if data.join(config::FILE).exists() {
        log::debug!("Migrate: config already present, skipped");
    } else if let Err(e) = migrate_config(&old, data) {
        log::warn!("Migrate: config failed: {e}");
    }

    if data.join(wordbook::FILE).exists() {
        log::debug!("Migrate: wordbook already present, skipped");
    } else if let Err(e) = migrate_wordbook(&old, data) {
        log::warn!("Migrate: wordbook failed: {e}");
    }
}

/// 旧目录和新目录同级，所以从新目录反推，不用再判一次平台
/// （Windows `%APPDATA%\com.tilex.desktop`、macOS `~/Library/Application Support/com.tilex.desktop`）。
fn legacy_dir(data: &Path) -> Option<PathBuf> {
    Some(data.parent()?.join(LEGACY_DIR))
}

fn migrate_config(old: &Path, data: &Path) -> Result<(), Error> {
    let bytes = fs::read(old.join(config::FILE))?;
    let kv: Map<String, Value> = serde_json::from_slice(&bytes)?;
    let mut config = config_from_legacy(&kv);
    config.normalize();
    fs::create_dir_all(data)?;
    fs::write(data.join(config::FILE), serde_json::to_vec_pretty(&config)?)?;
    log::info!(
        "Migrate: config done ({} translate, {} recognize services)",
        config.translate_services.len(),
        config.recognize_services.len()
    );
    Ok(())
}

fn str_of<'a>(kv: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    kv.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

fn config_from_legacy(kv: &Map<String, Value>) -> Config {
    let mut config = Config::default();

    // 旧版存的是小写 `zh_cn`，新版的界面语言只认 `zh_CN` / `en`（`config::LANGUAGES`）。
    if let Some(lang) = str_of(kv, "app_language") {
        config.general.language = if lang.eq_ignore_ascii_case("en") {
            "en".into()
        } else {
            "zh_CN".into()
        };
    }
    if let Some(v) = str_of(kv, "app_theme") {
        config.general.theme = v.into();
    }
    if let Some(v) = str_of(kv, "translate_source_language") {
        config.translate.source = v.into();
    }
    if let Some(v) = str_of(kv, "translate_target_language") {
        config.translate.target = v.into();
    }
    if let Some(v) = str_of(kv, "translate_detect_engine") {
        config.translate.detect_engine = v.into();
    }
    // 划词那份用得更多，两个都有值时以它为准（同 P9 的迁移规则）。
    if let Some(pos) = str_of(kv, "pop_result_pos").and_then(legacy_popup_pos) {
        config.translate.result_pos = pos.into();
    } else if let Some(pos) =
        str_of(kv, "screenshot_pos").and_then(config::screenshot_pos_to_result_pos)
    {
        config.translate.result_pos = pos.into();
    }

    if let Some(v) = kv.get("pop_button_enable").and_then(Value::as_bool) {
        config.selection.enabled = v;
    }
    if let Some(v) = str_of(kv, "pop_button_trigger") {
        config.selection.trigger = v.into();
    }
    if let Some(v) = kv.get("pop_button_exclude_native").and_then(Value::as_bool) {
        config.selection.exclude_native = v;
    }
    if let Some(v) = kv.get("pop_button_force_copy").and_then(Value::as_bool) {
        config.selection.force_copy = v;
    }
    if let Some(v) = kv.get("pop_button_blacklist").and_then(Value::as_str) {
        config.selection.blacklist = v.into();
    }
    if let Some(v) = str_of(kv, "pop_button_pos") {
        config.selection.button_pos = v.into();
    }
    if let Some(v) = kv.get("pop_button_distance").and_then(Value::as_i64) {
        config.selection.button_distance = v;
    }
    if let Some(v) = kv.get("hotkey_screenshot").and_then(Value::as_str) {
        config.screenshot.hotkey = v.into();
    }

    if let Some(list) = services_from_legacy(kv, "translate_service_list", "") {
        config.translate_services = list;
    }
    // OCR 实例的配置键带 `recognize_` 前缀，服务清单里却只有裸 id。
    if let Some(list) = services_from_legacy(kv, "recognize_service_list", "recognize_") {
        config.recognize_services = list;
    }
    config
}

/// 旧版划词弹窗位置（`bottom_right` 这套小写下划线）→ 新版 `translate.result_pos`。
/// 旧值是「相对选词点的四个象限」，新方案只有四条边，左右分量有意丢掉（同 P9）。
fn legacy_popup_pos(pos: &str) -> Option<&'static str> {
    match pos {
        "bottom_right" | "bottom_left" => Some("sel_bottom"),
        "top_right" | "top_left" => Some("sel_top"),
        _ => None,
    }
}

/// 清单里是 id 数组，数组顺序就是界面顺序。配置在顶层的 `<prefix><id>` 键下。
fn services_from_legacy(
    kv: &Map<String, Value>,
    list_key: &str,
    prefix: &str,
) -> Option<Vec<Service>> {
    let ids = kv.get(list_key)?.as_array()?;
    let list: Vec<Service> = ids
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|id| {
            let raw = kv.get(&format!("{prefix}{id}"))?.as_object()?;
            Some(service_from_legacy(id, raw))
        })
        .collect();
    (!list.is_empty()).then_some(list)
}

/// id 形如 `google`（第一个实例）或 `ai@k3x9`，`@` 前就是 kind。
fn service_from_legacy(id: &str, raw: &Map<String, Value>) -> Service {
    let kind = id.split('@').next().unwrap_or(id);
    let mut obj = if kind == "ai" {
        ai_config_from_legacy(raw)
    } else {
        let mut obj = raw.clone();
        // 旧版界面自己塞进配置的展示字段，不是服务配置，别跟着进 `Instance::extra`。
        for key in [
            "id",
            "name",
            "needApiKey",
            "helpUrl",
            "instanceName",
            "enable",
        ] {
            obj.remove(key);
        }
        // bing / deepl 的 `mode` 上有 `alias = "type"`，google 的没有，只有它要改名。
        if kind == "google"
            && let Some(mode) = obj.remove("type")
        {
            obj.insert("mode".into(), mode);
        }
        obj
    };
    obj.insert("kind".into(), kind.into());
    obj.insert("id".into(), id.into());
    // 旧版的启用判据也是「不等于 false 就算开」。
    obj.insert(
        "enabled".into(),
        (raw.get("enable") != Some(&Value::Bool(false))).into(),
    );
    if let Some(label) = str_of(raw, "instanceName") {
        obj.insert("label".into(), label.into());
    }

    match serde_json::from_value::<Service>(Value::Object(obj)) {
        Ok(service) => service,
        // 形状对不上也不能丢：原样收进 Unknown，界面不显示但写回时一个字段都不少。
        Err(e) => {
            log::warn!("Migrate: service {id} kept as unknown: {e}");
            Service::Unknown(Value::Object(raw.clone()))
        }
    }
}

/// 旧版 AI 实例的字段名和新版完全不同，而且旧版的规范化（`instructions.js` 的
/// `normalizeAiConfig`）是读时做的、不写盘，所以旧 profile 里还是原始的 `promptList`。
fn ai_config_from_legacy(raw: &Map<String, Value>) -> Map<String, Value> {
    let mut obj = Map::new();
    for (old_key, new_key) in [
        ("requestPath", "base_url"),
        ("apiKey", "api_key"),
        ("model", "model"),
    ] {
        obj.insert(
            new_key.into(),
            raw.get(old_key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
        );
    }
    // 协议名两版一致；认不出的按 openai_chat（`Protocol::from_name` 自己也这么兜底）。
    obj.insert(
        "protocol".into(),
        raw.get("apiFormat")
            .and_then(Value::as_str)
            .unwrap_or("openai_chat")
            .into(),
    );

    // 存过表单的实例已经有规范化后的 customInstructions，直接用；没有的才去折 promptList。
    let (instructions, references) = match raw.get("customInstructions").and_then(Value::as_str) {
        Some(s) => (
            s.to_owned(),
            raw.get("legacyReferenceInstructions")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        ),
        None => {
            let text = legacy_instructions(raw.get("promptList"));
            let refs = if has_legacy_placeholder(&text) {
                LEGACY_REFERENCES
            } else {
                ""
            };
            (text, refs.to_owned())
        }
    };
    obj.insert("custom_instructions".into(), instructions.into());
    if !references.is_empty() {
        obj.insert("legacy_reference_instructions".into(), references.into());
    }

    // 旧版把生成参数存成 JSON 字符串。解析不出对象就退回新版默认那四项。
    let args = raw
        .get("requestArguments")
        .and_then(|v| match v {
            Value::String(s) => serde_json::from_str::<Value>(s).ok(),
            other => Some(other.clone()),
        })
        .and_then(|v| match v {
            Value::Object(m) => Some(m),
            _ => None,
        })
        .unwrap_or_else(|| match serde_json::to_value(ai::Config::default()) {
            Ok(Value::Object(mut m)) => m
                .remove("request_arguments")
                .and_then(|v| match v {
                    Value::Object(m) => Some(m),
                    _ => None,
                })
                .unwrap_or_default(),
            _ => Map::new(),
        });
    obj.insert("request_arguments".into(), Value::Object(args));
    obj
}

/// 复刻 `instructions.js:46` 的 `legacyInstructions`。
fn legacy_instructions(prompt_list: Option<&Value>) -> String {
    let Some(value) = prompt_list else {
        return ai::DEFAULT_CUSTOM_INSTRUCTIONS.to_owned();
    };
    let Some(list) = value.as_array() else {
        return value.as_str().unwrap_or_default().to_owned();
    };
    let is_default = |v: &Value| {
        LEGACY_DEFAULT_PROMPTS.iter().any(|(role, content)| {
            v.get("role").and_then(Value::as_str) == Some(role)
                && v.get("content").and_then(Value::as_str) == Some(content)
        })
    };
    if list.len() == LEGACY_DEFAULT_PROMPTS.len() && list.iter().all(is_default) {
        return ai::DEFAULT_CUSTOM_INSTRUCTIONS.to_owned();
    }
    list.iter()
        .filter(|v| !is_default(v))
        .map(|v| match v {
            Value::String(s) => s.as_str(),
            other => other.get("content").and_then(Value::as_str).unwrap_or(""),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// `$text` / `$from` / `$to` / `$detect`，按词边界（`$total` 不算命中）。
fn has_legacy_placeholder(text: &str) -> bool {
    ["$text", "$from", "$to", "$detect"].iter().any(|p| {
        text.match_indices(p).any(|(i, _)| {
            text[i + p.len()..]
                .chars()
                .next()
                .is_none_or(|c| !c.is_alphanumeric() && c != '_')
        })
    })
}

fn migrate_wordbook(old: &Path, data: &Path) -> Result<(), Error> {
    let source = old.join(wordbook::FILE);
    if !source.is_file() {
        return Ok(());
    }
    // 从副本读：既不碰原库，也绕开「只读连接打开 WAL 库要写 -shm」那个坑。
    // `-wal` 里可能还有没合并的记录（本机那份就有），三个文件必须一起复制。
    let tmp = std::env::temp_dir().join(format!("tilex-migrate-{}", std::process::id()));
    fs::create_dir_all(&tmp)?;
    let copy = tmp.join(wordbook::FILE);
    let result = (|| -> Result<usize, Error> {
        for suffix in ["", "-wal", "-shm"] {
            let from = source.with_file_name(format!("{}{suffix}", wordbook::FILE));
            if from.is_file() {
                fs::copy(
                    &from,
                    copy.with_file_name(format!("{}{suffix}", wordbook::FILE)),
                )?;
            }
        }
        let conn = Connection::open(data.join(wordbook::FILE))?;
        conn.execute_batch(wordbook::SCHEMA)?;
        conn.execute(
            "ATTACH DATABASE ?1 AS legacy",
            [copy.to_string_lossy().as_ref()],
        )?;
        // id 原样搬，否则 source_id 的自引用会断。旧表没有 service / updated_at，
        // translation 和 created_at 可空；type 在旧表上没有 CHECK，认不出的按 sentence。
        let n = conn.execute(
            "INSERT INTO entries (id, kind, text, translation, detail, source_id, service, created_at, updated_at, deleted)
             SELECT id,
                    CASE WHEN type = 'word' THEN 'word' ELSE 'sentence' END,
                    text,
                    COALESCE(translation, ''),
                    detail,
                    source_id,
                    '',
                    COALESCE(created_at, 0),
                    COALESCE(created_at, 0),
                    COALESCE(deleted, 0)
             FROM legacy.entries",
            [],
        )?;
        conn.execute_batch("DETACH DATABASE legacy")?;
        Ok(n)
    })();
    // ignore: 临时副本删不掉不影响迁移结果，系统的临时目录迟早会被清掉
    let _ = fs::remove_dir_all(&tmp);
    log::info!("Migrate: wordbook done ({} entries)", result?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn legacy_kv() -> Map<String, Value> {
        json!({
            "app_language": "zh_cn",
            "app_theme": "light",
            "translate_source_language": "auto",
            "translate_target_language": "zh_cn",
            "translate_detect_engine": "local",
            "pop_result_pos": "bottom_right",
            "screenshot_pos": "box_bottom_left",
            "pop_button_enable": true,
            "pop_button_trigger": "hover",
            "pop_button_exclude_native": true,
            "pop_button_force_copy": true,
            "pop_button_blacklist": "notepad.exe",
            "pop_button_pos": "bottom_left",
            "pop_button_distance": 12,
            "hotkey_screenshot": "Alt+S",
            "app_font": "dropped",
            "dev_mode": true,
            "translate_service_list": ["google", "bing@v1", "ai@k1", "transmart@t1"],
            "recognize_service_list": ["wechat"],
            "google": {"type": "api", "api_key": "g-key", "enable": true},
            "bing@v1": {"instanceName": "我的 Bing", "type": "api", "auth_key": "b-key", "region": "eastasia", "enable": false},
            "ai@k1": {
                "instanceName": "本地",
                "requestPath": "http://127.0.0.1:11434/v1",
                "model": "qwen3",
                "apiKey": "sk-x",
                "apiFormat": "openai_chat",
                "stream": false,
                "promptList": [{"role": "user", "content": "翻译成 $to，别啰嗦"}],
                "requestArguments": "{\"temperature\": 0.7}",
                "id": "ai",
                "name": "AI",
                "needApiKey": true,
                "helpUrl": "https://example.test"
            },
            "transmart@t1": {"instanceName": "", "enable": true},
            "recognize_wechat": {}
        })
        .as_object()
        .unwrap()
        .clone()
    }

    #[test]
    fn scalars_land_on_the_new_keys() {
        let c = config_from_legacy(&legacy_kv());
        assert_eq!(c.general.language, "zh_CN"); // 旧版存的是小写 zh_cn
        assert_eq!(c.general.theme, "light");
        assert_eq!(c.translate.source, "auto");
        assert_eq!(c.translate.target, "zh_cn");
        assert_eq!(c.translate.detect_engine, "local");
        assert_eq!(c.selection.trigger, "hover");
        assert_eq!(c.selection.blacklist, "notepad.exe");
        assert_eq!(c.selection.button_pos, "bottom_left");
        assert_eq!(c.selection.button_distance, 12);
        assert_eq!(c.screenshot.hotkey, "Alt+S");
    }

    #[test]
    fn english_ui_keeps_english() {
        let mut kv = legacy_kv();
        kv.insert("app_language".into(), "en".into());
        assert_eq!(config_from_legacy(&kv).general.language, "en");
    }

    #[test]
    fn popup_position_wins_over_screenshot_position() {
        let c = config_from_legacy(&legacy_kv());
        assert_eq!(c.translate.result_pos, "sel_bottom");

        // 划词那份认不出时才看截图那份
        let mut kv = legacy_kv();
        kv.insert("pop_result_pos".into(), "nonsense".into());
        kv.insert("screenshot_pos".into(), "cursor_top_left".into());
        assert_eq!(
            config_from_legacy(&kv).translate.result_pos,
            "cursor_top_left"
        );

        // 两份都认不出就是默认值
        let mut kv = legacy_kv();
        kv.remove("pop_result_pos");
        kv.remove("screenshot_pos");
        assert_eq!(
            config_from_legacy(&kv).translate.result_pos,
            Config::default().translate.result_pos
        );
    }

    #[test]
    fn top_positions_map_to_sel_top() {
        for pos in ["top_left", "top_right"] {
            let mut kv = legacy_kv();
            kv.insert("pop_result_pos".into(), pos.into());
            assert_eq!(config_from_legacy(&kv).translate.result_pos, "sel_top");
        }
    }

    #[test]
    fn services_keep_order_and_keys() {
        let c = config_from_legacy(&legacy_kv());
        assert_eq!(c.translate_services.len(), 4);
        let Service::Google(g) = &c.translate_services[0] else {
            panic!("first is not google");
        };
        assert_eq!(g.id, "google");
        assert!(g.enabled);
        assert_eq!(g.config.mode, "api"); // 旧 `type` 改名成 `mode`
        assert_eq!(g.config.api_key, "g-key");

        let Service::Bing(b) = &c.translate_services[1] else {
            panic!("second is not bing");
        };
        assert_eq!(b.id, "bing@v1");
        assert!(!b.enabled); // enable: false 要保住
        assert_eq!(b.label, "我的 Bing");
        assert_eq!(b.config.auth_key, "b-key");
        assert_eq!(b.config.region, "eastasia");

        let Service::Transmart(t) = &c.translate_services[3] else {
            panic!("fourth is not transmart");
        };
        assert_eq!(t.id, "transmart@t1");
        assert_eq!(t.label, ""); // 空的 instanceName 不占位

        assert_eq!(c.recognize_services.len(), 1);
        let Service::Wechat(w) = &c.recognize_services[0] else {
            panic!("not wechat");
        };
        assert_eq!(w.id, "wechat"); // 配置键是 recognize_wechat，id 仍是裸 id
    }

    #[test]
    fn ai_instance_is_fully_mapped() {
        let c = config_from_legacy(&legacy_kv());
        let Service::Ai(a) = &c.translate_services[2] else {
            panic!("third is not ai");
        };
        assert_eq!(a.id, "ai@k1");
        assert_eq!(a.label, "本地");
        assert_eq!(a.config.base_url, "http://127.0.0.1:11434/v1");
        assert_eq!(a.config.api_key, "sk-x");
        assert_eq!(a.config.model, "qwen3");
        assert_eq!(a.config.protocol, "openai_chat");
        assert_eq!(a.config.custom_instructions, "翻译成 $to，别啰嗦");
        // 折出来的要求里有 $to，才带上占位符说明
        assert_eq!(a.config.legacy_reference_instructions, LEGACY_REFERENCES);
        assert_eq!(
            a.config.request_arguments.get("temperature"),
            Some(&json!(0.7))
        );
        // 旧版界面自己的展示字段不许跟进来
        for key in [
            "id",
            "name",
            "needApiKey",
            "helpUrl",
            "stream",
            "promptList",
        ] {
            assert!(!a.extra.contains_key(key), "{key} leaked into extra");
        }
    }

    #[test]
    fn untouched_prompts_become_the_new_default() {
        let list: Vec<Value> = LEGACY_DEFAULT_PROMPTS
            .iter()
            .map(|(role, content)| json!({"role": role, "content": content}))
            .collect();
        assert_eq!(
            legacy_instructions(Some(&Value::Array(list))),
            ai::DEFAULT_CUSTOM_INSTRUCTIONS
        );
        // 没有 promptList 的实例同样用默认值
        assert_eq!(legacy_instructions(None), ai::DEFAULT_CUSTOM_INSTRUCTIONS);
    }

    #[test]
    fn custom_prompts_drop_the_default_ones_and_join() {
        let list = json!([
            {"role": "system", "content": LEGACY_DEFAULT_PROMPTS[0].1},
            {"role": "user", "content": "第一条"},
            "第二条"
        ]);
        assert_eq!(legacy_instructions(Some(&list)), "第一条\n\n第二条");
    }

    #[test]
    fn placeholder_detection_respects_word_boundaries() {
        assert!(has_legacy_placeholder("Translate into $to:"));
        assert!(has_legacy_placeholder("$text"));
        assert!(!has_legacy_placeholder("$total 不是占位符"));
        assert!(!has_legacy_placeholder("没有占位符"));
    }

    #[test]
    fn saved_instructions_win_over_prompt_list() {
        let raw = json!({
            "customInstructions": "用户自己写的",
            "legacyReferenceInstructions": "",
            "promptList": [{"role": "user", "content": "旧的 $text"}]
        });
        let obj = ai_config_from_legacy(raw.as_object().unwrap());
        assert_eq!(obj["custom_instructions"], json!("用户自己写的"));
        assert!(!obj.contains_key("legacy_reference_instructions"));
    }

    #[test]
    fn broken_request_arguments_fall_back_to_defaults() {
        let raw = json!({"requestArguments": "{ 这不是 JSON"});
        let obj = ai_config_from_legacy(raw.as_object().unwrap());
        assert_eq!(
            obj["request_arguments"],
            serde_json::to_value(ai::Config::default().request_arguments).unwrap()
        );
    }

    #[test]
    fn unknown_kind_keeps_every_field() {
        let raw = json!({"api_key": "secret", "enable": true});
        let service = service_from_legacy("whatever@x", raw.as_object().unwrap());
        let Service::Unknown(v) = service else {
            panic!("should be unknown");
        };
        assert_eq!(v["api_key"], json!("secret"));
    }

    #[test]
    fn missing_service_entry_is_skipped_not_faked() {
        let mut kv = legacy_kv();
        kv.remove("bing@v1");
        let c = config_from_legacy(&kv);
        assert_eq!(c.translate_services.len(), 3);
        assert!(
            !c.translate_services
                .iter()
                .any(|s| matches!(s, Service::Bing(_)))
        );
    }

    #[test]
    fn empty_service_list_keeps_the_defaults() {
        let mut kv = legacy_kv();
        kv.insert("translate_service_list".into(), json!([]));
        let c = config_from_legacy(&kv);
        assert_eq!(c.translate_services, Config::default().translate_services);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tilex-migrate-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 旧版建表语句，照抄 `src/utils/wordbook.js:13-22`。
    const LEGACY_SCHEMA: &str = "CREATE TABLE entries (
        id INTEGER PRIMARY KEY, type TEXT NOT NULL, text TEXT NOT NULL, translation TEXT,
        detail TEXT, source_id INTEGER, created_at INTEGER, deleted INTEGER DEFAULT 0)";

    fn seed_legacy_db(dir: &Path) {
        let conn = Connection::open(dir.join(wordbook::FILE)).unwrap();
        conn.execute_batch(LEGACY_SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO entries (id, type, text, translation, detail, source_id, created_at, deleted)
             VALUES (7, 'word', 'hello', NULL, '{\"kind\":\"word\"}', NULL, 1700000000000, 0),
                    (8, 'sentence', 'hello world', '你好世界', NULL, 7, NULL, 0),
                    (9, 'weird', 'x', 'y', NULL, NULL, 1, 1)",
            [],
        )
        .unwrap();
    }

    /// 迁完的那张表里要断言的列（`text` 不在其中：它逐字照搬，没有映射规则）。
    struct Row {
        id: i64,
        kind: String,
        translation: String,
        source_id: Option<i64>,
        service: String,
        created_at: i64,
        updated_at: i64,
        deleted: i64,
    }

    #[test]
    fn wordbook_rows_are_mapped_column_by_column() {
        let root = temp_dir("db");
        let old = root.join("old");
        let new = root.join("new");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        seed_legacy_db(&old);

        migrate_wordbook(&old, &new).unwrap();

        let conn = Connection::open(new.join(wordbook::FILE)).unwrap();
        let mut stmt = conn
            .prepare("SELECT id, kind, text, translation, source_id, service, created_at, updated_at, deleted FROM entries ORDER BY id")
            .unwrap();
        let rows: Vec<Row> = stmt
            .query_map([], |r| {
                Ok(Row {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    translation: r.get(3)?,
                    source_id: r.get(4)?,
                    service: r.get(5)?,
                    created_at: r.get(6)?,
                    updated_at: r.get(7)?,
                    deleted: r.get(8)?,
                })
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();

        assert_eq!(rows.len(), 3);
        // id 原样搬，source_id 的自引用才不会断
        assert_eq!(rows[0].id, 7);
        assert_eq!(rows[1].source_id, Some(7));
        assert_eq!(rows[0].translation, ""); // translation 的 NULL 变空串
        assert_eq!(rows[0].created_at, 1700000000000);
        assert_eq!(rows[0].updated_at, 1700000000000); // 旧表没有 updated_at，取 created_at
        assert_eq!(rows[0].service, ""); // 旧表没有 service
        assert_eq!(rows[1].created_at, 0); // created_at 的 NULL 变 0
        assert_eq!(rows[2].kind, "sentence"); // 认不出的 type 按 sentence，不让 CHECK 拒掉整条
        assert_eq!(rows[2].deleted, 1); // deleted 保留

        // 旧库原封未动
        let old_conn = Connection::open(old.join(wordbook::FILE)).unwrap();
        let n: i64 = old_conn
            .query_row("SELECT count(*) FROM entries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 3);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn run_is_idempotent_and_never_touches_an_existing_profile() {
        let root = temp_dir("run");
        let old = root.join(LEGACY_DIR);
        let new = root.join("TiLex");
        fs::create_dir_all(&old).unwrap();
        fs::write(
            old.join(config::FILE),
            serde_json::to_vec(&legacy_kv()).unwrap(),
        )
        .unwrap();
        seed_legacy_db(&old);

        run(&new);
        let first = fs::read_to_string(new.join(config::FILE)).unwrap();
        assert!(first.contains("bing@v1"));

        // 再跑一次不许覆盖用户后来改过的配置
        fs::write(new.join(config::FILE), "{\"version\":1}").unwrap();
        run(&new);
        assert_eq!(
            fs::read_to_string(new.join(config::FILE)).unwrap(),
            "{\"version\":1}"
        );

        // 生词本也一样：第二次不许重复插入
        let conn = Connection::open(new.join(wordbook::FILE)).unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM entries", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 3);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_broken_legacy_config_does_not_stop_startup() {
        let root = temp_dir("broken");
        let old = root.join(LEGACY_DIR);
        let new = root.join("TiLex");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join(config::FILE), "{ 这不是 JSON").unwrap();

        run(&new); // 不许 panic
        assert!(!new.join(config::FILE).exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn no_legacy_profile_is_a_no_op() {
        let root = temp_dir("none");
        let new = root.join("TiLex");
        fs::create_dir_all(&new).unwrap();
        run(&new);
        assert!(!new.join(config::FILE).exists());
        let _ = fs::remove_dir_all(&root);
    }
}
