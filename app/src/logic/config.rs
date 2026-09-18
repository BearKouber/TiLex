//! 配置（design §2.1）：一个文件 `<数据目录>/config.json`，一个强类型结构体，内存里一份。
//! 只有 UI 线程写：先改副本、写盘成功再替换内存；失败返回 `Err`，内存不变。
//! 其他线程在请求开始时 `snapshot()` 拷一份，之后不再读全局。

use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::error::Error;
use crate::service::{ai, baidu, bing, deepl, google, umi};

pub use crate::service::ai::Config as AiConfig;
pub use crate::service::baidu::Config as BaiduConfig;
pub use crate::service::bing::Config as BingConfig;
pub use crate::service::deepl::Config as DeeplConfig;
pub use crate::service::google::Config as GoogleConfig;
pub use crate::service::umi::Config as UmiConfig;

/// 界面语言。顺序就是设置页下拉框的顺序；值是 `ui/i18n/` 下的目录名，`en` 是 msgid 原文。
pub const LANGUAGES: [&str; 2] = ["zh_CN", "en"];
const FILE: &str = "config.json";
/// 写盘超过这个时间记一条 warn（design §2.1：真出现卡顿再挪到后台线程）。
const SLOW_WRITE: Duration = Duration::from_millis(50);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub version: u32,
    pub general: General,
    pub translate: Translate,
    pub selection: Selection,
    pub screenshot: Screenshot,
    /// 有序：数组顺序就是界面上的顺序。删一个服务就是删一个元素。
    pub translate_services: Vec<Service>,
    pub recognize_services: Vec<Service>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    pub language: String,
    pub theme: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Translate {
    pub source: String,
    pub target: String,
    pub detect_engine: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Selection {
    pub enabled: bool,
    pub trigger: String,
    pub exclude_native: bool,
    pub force_copy: bool,
    pub blacklist: String,
    pub button_pos: String,
    /// 浮标离选区的间距，物理像素，0–20（`normalize` 夹紧）。手改成字符串、小数等非法值时用默认 10，
    /// 不让整份配置因为这一项被当成损坏。
    #[serde(deserialize_with = "distance")]
    pub button_distance: i64,
    pub result_pos: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Screenshot {
    pub hotkey: String,
    pub result_pos: String,
}

/// 一个服务实例。`kind` 决定形状；各服务自己的字段在 `service::xxx::Config` 里。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Service {
    Google(Instance<google::Config>),
    Bing(Instance<bing::Config>),
    Deepl(Instance<deepl::Config>),
    Baidu(Instance<baidu::Config>),
    Transmart(Instance<NoSettings>),
    Ai(Instance<ai::Config>),
    Wechat(Instance<NoSettings>),
    Umi(Instance<umi::Config>),
    /// 认不出的 kind（例如从新版降级回来）或字段对不上的：原样保留、原样写回，界面不显示。
    /// 不因为认不出就丢掉用户的 API key。
    #[serde(untagged)]
    Unknown(serde_json::Value),
}

/// 服务实例的公共字段 + 服务自己的配置。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Instance<C> {
    /// 实例 id，形如 `ai@k3x9`；同一服务可以有多个实例。
    pub id: String,
    /// 启用判据是 `enabled != false`（旧规范的坑），缺省为 true。
    pub enabled: bool,
    /// 用户起的名字；空 = 用服务默认名。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub label: String,
    #[serde(flatten)]
    pub config: C,
    /// 这一版不认识的字段（新版加的、以后的批次加的）原样写回：交替运行不同版本时不丢 key 之类的新字段。
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// 没有可配项的服务（微信 OCR）。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NoSettings {}

impl Default for Config {
    fn default() -> Self {
        Self {
            version: 1,
            general: General::default(),
            translate: Translate::default(),
            selection: Selection::default(),
            screenshot: Screenshot::default(),
            translate_services: vec![Service::Google(Instance::new("google"))],
            recognize_services: vec![Service::Wechat(Instance::new("wechat"))],
        }
    }
}

impl Default for General {
    fn default() -> Self {
        Self {
            language: LANGUAGES[0].into(),
            theme: "system".into(),
        }
    }
}

impl Default for Translate {
    fn default() -> Self {
        Self {
            source: "auto".into(),
            target: "zh_cn".into(),
            detect_engine: "local".into(),
        }
    }
}

impl Default for Selection {
    fn default() -> Self {
        Self {
            enabled: false,
            trigger: "hover".into(),
            exclude_native: true,
            force_copy: false,
            blacklist: String::new(),
            button_pos: "BottomLeft".into(),
            button_distance: 10,
            result_pos: "BottomRight".into(),
        }
    }
}

impl Default for Screenshot {
    fn default() -> Self {
        Self {
            hotkey: String::new(),
            // 截图浮窗的位置是另一套值域（`box_*` / `cursor_*`，见 `ui::settings::SCREENSHOT_POS`），
            // 不是划词浮窗那套驼峰角名。默认同旧版：面板左上角对准选区左下角。
            result_pos: "box_bottom_left".into(),
        }
    }
}

impl<C: Default> Default for Instance<C> {
    fn default() -> Self {
        Self::new("")
    }
}

impl<C: Default> Instance<C> {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.to_owned(),
            enabled: true,
            label: String::new(),
            config: C::default(),
            extra: Map::new(),
        }
    }
}

static INSTANCE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 新实例的 id，形如 `google@18ff0c3e1a2b3c4d0`（旧版是 `<kind>@<随机 base36>`）。
/// 纳秒时间戳 + 进程内计数器，不引第三方随机数库；同一毫秒内连加多个也不会撞。
pub fn new_instance_id(kind: &str) -> String {
    let count = INSTANCE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{kind}@{nanos:x}{count:x}")
}

/// 旧 `normalizePopButtonDistance` / `button_distance()` 的契约：整数（含 JSON 的 `4.0`）保留，
/// 越界的由 `normalize` 夹紧；缺失、null、布尔、字符串、数组、对象、小数都用默认值。
fn distance<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(value
        .as_f64()
        .filter(|n| n.is_finite() && n.fract() == 0.0)
        .map_or(Selection::default().button_distance, |n| {
            n.clamp(0.0, 20.0) as i64
        }))
}

impl Config {
    /// 反序列化和每次修改之后统一把取值拉回合法范围。
    pub fn normalize(&mut self) {
        if !LANGUAGES.contains(&self.general.language.as_str()) {
            self.general.language = General::default().language;
        }
        self.selection.button_distance = self.selection.button_distance.clamp(0, 20);
    }
}

/// 一份配置文件和它在内存里的副本。全局只有一份（`init`），测试里各建各的。
pub struct Store {
    path: PathBuf,
    current: RwLock<Config>,
}

impl Store {
    /// 读 `<dir>/config.json`。没有就写一份默认值；
    /// 读出来解析不了就改名成 `config.json.bad-<秒>` 备份（不覆盖原文件），用默认值启动，
    /// 并把备份路径返回给界面提示一次。
    pub fn open(dir: &Path) -> Result<(Store, Option<PathBuf>), Error> {
        fs::create_dir_all(dir)?;
        let path = dir.join(FILE);
        let (config, backup) = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<Config>(&bytes) {
                Ok(mut config) => {
                    config.normalize();
                    (config, None)
                }
                Err(e) => {
                    let secs = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let backup = dir.join(format!("{FILE}.bad-{secs}"));
                    // 改名失败就不能继续：之后的保存会覆盖掉这份读不出来的文件。
                    fs::rename(&path, &backup)?;
                    // 只记类别和位置：serde 的 invalid type 错误会带上字段值（可能是 API key）。
                    log::warn!(
                        "Config: unreadable ({:?} at {}:{}), moved aside, using defaults",
                        e.classify(),
                        e.line(),
                        e.column()
                    );
                    let config = Config::default();
                    write_atomic(&path, &config)?;
                    (config, Some(backup))
                }
            },
            Err(e) if e.kind() == ErrorKind::NotFound => {
                log::info!("Config: no file, writing defaults");
                let config = Config::default();
                write_atomic(&path, &config)?;
                (config, None)
            }
            Err(e) => return Err(e.into()),
        };
        Ok((
            Store {
                path,
                current: RwLock::new(config),
            },
            backup,
        ))
    }

    /// 当前配置的一份拷贝。
    pub fn snapshot(&self) -> Config {
        // 锁中毒也照读：内存里永远是一份完整的配置（只整份替换），不存在改了一半的状态。
        self.current
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// 改副本 → 写盘 → 成功才替换内存。失败时内存不变，调用方把错误显示给用户并恢复控件。
    pub fn update(&self, change: impl FnOnce(&mut Config)) -> Result<(), Error> {
        let mut next = self.snapshot();
        change(&mut next);
        next.normalize();
        write_atomic(&self.path, &next)?;
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = next;
        Ok(())
    }
}

/// 序列化 → 写 `config.json.tmp` 并落盘 → 改名覆盖（std 的 rename 在 Windows 上会替换已存在的文件）。
fn write_atomic(path: &Path, config: &Config) -> Result<(), Error> {
    let started = Instant::now();
    let bytes = serde_json::to_vec_pretty(config)?;
    let tmp = path.with_extension("json.tmp");
    let result = File::create(&tmp)
        .and_then(|mut f| f.write_all(&bytes).and_then(|()| f.sync_all()))
        .and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp); // ignore: 临时文件删不掉只是留个残渣，下次写会覆盖它
    }
    let took = started.elapsed();
    if took > SLOW_WRITE {
        log::warn!("Config: slow write {} ms", took.as_millis());
    }
    Ok(result?)
}

static STORE: OnceLock<Store> = OnceLock::new();

/// `init` 看到的启动情形，决定要不要自己弹设置窗口。
pub enum Startup {
    /// 配置文件读出来了，静默启动，只进托盘。
    Normal,
    /// 数据目录里还没有配置文件：首次运行，弹设置窗口（F11）。
    FirstRun,
    /// 配置文件读不出来，已改名备份、换成默认值：弹设置窗口并提示备份文件名。
    Recovered(PathBuf),
}

/// 启动时调一次。
pub fn init(dir: &Path) -> Result<Startup, Error> {
    // 在 `Store::open` 之前问：它发现文件不存在就会把默认配置写下去，之后再问就永远是"存在"。
    let first_run = !dir.join(FILE).exists();
    let (store, backup) = Store::open(dir)?;
    STORE
        .set(store)
        .map_err(|_| Error::Platform("config already initialized".into()))?;
    Ok(match backup {
        Some(path) => Startup::Recovered(path),
        None if first_run => Startup::FirstRun,
        None => Startup::Normal,
    })
}

/// 全局配置的拷贝。没初始化时返回默认值。
pub fn snapshot() -> Config {
    STORE.get().map(Store::snapshot).unwrap_or_default()
}

/// 修改全局配置，见 [`Store::update`]。
pub fn update(change: impl FnOnce(&mut Config)) -> Result<(), Error> {
    STORE
        .get()
        .ok_or_else(|| Error::Platform("config not initialized".into()))?
        .update(change)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tilex-config-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn disk(dir: &Path) -> Config {
        serde_json::from_slice(&fs::read(dir.join(FILE)).unwrap()).unwrap()
    }

    #[test]
    fn missing_file_writes_defaults() {
        let dir = temp_dir("missing");
        let (store, backup) = Store::open(&dir).unwrap();
        assert!(backup.is_none());
        assert_eq!(store.snapshot(), Config::default());
        assert_eq!(disk(&dir), Config::default());
        fs::remove_dir_all(dir).unwrap();
    }

    // 旧 config.rs::save_failure_rolls_back_cache_and_does_not_publish_success 的新版。
    #[test]
    fn write_failure_keeps_memory_and_allows_retry() {
        let dir = temp_dir("rollback");
        let (store, _) = Store::open(&dir).unwrap();
        store.update(|c| c.selection.enabled = true).unwrap();

        // 保存路径上放一个目录：改名必然失败。
        fs::remove_file(dir.join(FILE)).unwrap();
        fs::create_dir(dir.join(FILE)).unwrap();
        let failed = store.update(|c| {
            c.selection.enabled = false;
            c.general.language = "en".into();
        });
        let message = failed.unwrap_err().to_string();
        assert!(!message.trim().is_empty());
        let current = store.snapshot();
        assert!(current.selection.enabled);
        assert_eq!(current.general.language, "zh_CN");
        assert!(!dir.join("config.json.tmp").exists(), "临时文件要清掉");

        fs::remove_dir(dir.join(FILE)).unwrap();
        store.update(|c| c.general.language = "en".into()).unwrap();
        assert_eq!(disk(&dir).general.language, "en");
        assert!(disk(&dir).selection.enabled);
        fs::remove_dir_all(dir).unwrap();
    }

    // B0 验收"配置文件设为只读后改设置 → 报错且内存不变"。
    // 只在 Windows 成立：Unix 上改名覆盖只看目录权限，只读文件照样被替换。
    #[test]
    fn read_only_file_is_an_error_on_windows() {
        if !cfg!(windows) {
            return;
        }
        let dir = temp_dir("readonly");
        let (store, _) = Store::open(&dir).unwrap();
        let path = dir.join(FILE);
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms.clone()).unwrap();

        assert!(store.update(|c| c.general.language = "en".into()).is_err());
        assert_eq!(store.snapshot().general.language, "zh_CN");
        assert_eq!(disk(&dir).general.language, "zh_CN");

        #[allow(
            clippy::permissions_set_readonly_false,
            reason = "测试收尾，恢复可写好删除"
        )]
        perms.set_readonly(false);
        fs::set_permissions(&path, perms).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    // 旧 config.rs::reload_replaces_snapshot_and_preserves_cache_on_invalid_json 的新版：
    // 读不出来的文件改名保留，不覆盖。
    #[test]
    fn corrupt_file_is_moved_aside_not_overwritten() {
        let dir = temp_dir("corrupt");
        fs::write(dir.join(FILE), b"{\"general\": ").unwrap();
        let (store, backup) = Store::open(&dir).unwrap();
        let backup = backup.unwrap();
        assert_eq!(fs::read(&backup).unwrap(), b"{\"general\": ");
        assert!(
            backup
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("config.json.bad-")
        );
        assert_eq!(store.snapshot(), Config::default());
        assert_eq!(disk(&dir), Config::default());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_file_counts_as_corrupt() {
        let dir = temp_dir("empty");
        fs::write(dir.join(FILE), b"").unwrap();
        let (_, backup) = Store::open(&dir).unwrap();
        assert!(backup.unwrap().exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_fields_use_defaults() {
        let config: Config =
            serde_json::from_value(json!({"selection": {"enabled": true}})).unwrap();
        assert!(config.selection.enabled);
        assert_eq!(config.selection.button_distance, 10);
        assert_eq!(config.general, General::default());
        assert_eq!(
            config.translate_services,
            Config::default().translate_services
        );
    }

    #[test]
    fn normalize_pulls_values_into_range() {
        let mut config = Config::default();
        for (value, expected) in [
            (0, 0),
            (4, 4),
            (10, 10),
            (20, 20),
            (-1, 0),
            (-100, 0),
            (21, 20),
            (1000, 20),
        ] {
            config.selection.button_distance = value;
            config.normalize();
            assert_eq!(
                config.selection.button_distance, expected,
                "distance {value}"
            );
        }
        config.general.language = "fr".into();
        config.normalize();
        assert_eq!(config.general.language, "zh_CN");
        config.general.language = "en".into();
        config.normalize();
        assert_eq!(config.general.language, "en");
    }

    #[test]
    fn update_normalizes() {
        let dir = temp_dir("update-normalize");
        let (store, _) = Store::open(&dir).unwrap();
        store.update(|c| c.selection.button_distance = 99).unwrap();
        assert_eq!(store.snapshot().selection.button_distance, 20);
        assert_eq!(disk(&dir).selection.button_distance, 20);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unknown_service_kind_round_trips() {
        let future = json!({
            "id": "ai@k3x9", "kind": "future_ai", "enabled": false,
            "api_key": "sk-secret", "nested": {"a": [1, 2]}
        });
        let known = json!({"id": "google", "kind": "google", "enabled": true});
        let broken_known = json!({"id": "wechat", "kind": "wechat", "enabled": "yes"});
        let input =
            json!({"translate_services": [known, future], "recognize_services": [broken_known]});

        let config: Config = serde_json::from_value(input).unwrap();
        assert_eq!(
            config.translate_services[0],
            Service::Google(Instance::new("google"))
        );
        assert!(matches!(config.translate_services[1], Service::Unknown(_)));
        assert!(matches!(config.recognize_services[0], Service::Unknown(_)));

        let out = serde_json::to_value(&config).unwrap();
        assert_eq!(out["translate_services"][0], known);
        assert_eq!(out["translate_services"][1], future);
        assert_eq!(out["recognize_services"][0], broken_known);
    }

    // B0 审查 P2：认得的服务里不认得的字段（以后的批次加的）也要原样写回。
    #[test]
    fn unknown_fields_of_known_services_round_trip() {
        let ai = json!({
            "id": "ai@k3x9", "kind": "ai", "enabled": true, "label": "DeepSeek",
            "base_url": "https://api.deepseek.com", "api_key": "sk-secret", "model": "deepseek-chat",
            "protocol": "openai_chat", "custom_instructions": "",
            "request_arguments": {"temperature": 0.3},
            "icon": "deepseek", "future_field": {"nested": [1, 2]},
        });
        let google = json!({"id": "google", "kind": "google", "enabled": false, "mode": "api",
                            "api_key": "g-key", "later": true});
        let wechat = json!({"id": "wechat", "kind": "wechat", "enabled": true, "path": "D:/微信"});
        let input = json!({"translate_services": [ai, google], "recognize_services": [wechat]});
        let config: Config = serde_json::from_value(input).unwrap();

        let Service::Ai(instance) = &config.translate_services[0] else {
            panic!(
                "ai entry not recognized: {:?}",
                config.translate_services[0]
            );
        };
        assert_eq!(instance.label, "DeepSeek");
        assert_eq!(instance.config.api_key, "sk-secret");
        assert_eq!(instance.config.custom_instructions, "", "显式空要求保留");
        assert_eq!(instance.extra.len(), 2);
        let Service::Google(g) = &config.translate_services[1] else {
            panic!("google entry not recognized");
        };
        assert!(!g.enabled);
        assert_eq!(g.config.mode, "api");

        let out = serde_json::to_value(&config).unwrap();
        assert_eq!(out["translate_services"][0], ai);
        assert_eq!(out["translate_services"][1], google);
        assert_eq!(out["recognize_services"][0], wechat);
    }

    #[test]
    fn umi_service_config_round_trip() {
        let umi = json!({
            "id": "umi@123",
            "kind": "umi",
            "enabled": true,
            "url": "http://192.168.1.100:1224/api/ocr",
            "extra_field": "preserved"
        });
        let input = json!({
            "recognize_services": [umi]
        });
        let config: Config = serde_json::from_value(input).unwrap();
        let Service::Umi(instance) = &config.recognize_services[0] else {
            panic!(
                "umi entry not recognized: {:?}",
                config.recognize_services[0]
            );
        };
        assert_eq!(instance.id, "umi@123");
        assert!(instance.enabled);
        assert_eq!(instance.config.url, "http://192.168.1.100:1224/api/ocr");
        assert_eq!(instance.extra.get("extra_field"), Some(&json!("preserved")));

        let out = serde_json::to_value(&config).unwrap();
        assert_eq!(out["recognize_services"][0], umi);
    }

    #[test]
    fn bing_and_deepl_service_config_round_trip() {
        let bing = json!({
            "id": "bing@1",
            "kind": "bing",
            "enabled": true,
            "mode": "api",
            "auth_key": "azure-key",
            "region": "eastasia",
            "custom_url": "https://api.cognitive.microsofttranslator.com",
            "extra_field": 42
        });
        let deepl = json!({
            "id": "deepl@1",
            "kind": "deepl",
            "enabled": false,
            "mode": "deeplx",
            "custom_url": "http://127.0.0.1:1188/translate",
            "extra_field": "preserved"
        });
        let input = json!({
            "translate_services": [bing, deepl]
        });
        let config: Config = serde_json::from_value(input).unwrap();

        let Service::Bing(b) = &config.translate_services[0] else {
            panic!("expected bing");
        };
        assert_eq!(b.config.auth_key, "azure-key");
        assert_eq!(b.config.region, "eastasia");
        assert_eq!(b.extra.get("extra_field"), Some(&json!(42)));

        let Service::Deepl(d) = &config.translate_services[1] else {
            panic!("expected deepl");
        };
        assert!(!d.enabled);
        assert_eq!(d.config.mode, "deeplx");
        assert_eq!(d.config.custom_url, "http://127.0.0.1:1188/translate");

        let out = serde_json::to_value(&config).unwrap();
        assert_eq!(out["translate_services"][0], bing);
        assert_eq!(out["translate_services"][1], deepl);
    }

    #[test]
    fn baidu_and_transmart_service_config_round_trip() {
        let baidu = json!({
            "id": "baidu@1",
            "kind": "baidu",
            "enabled": true,
            "appid": "baidu-app-id",
            "secret": "baidu-secret-key",
            "extra_field": 100
        });
        let transmart = json!({
            "id": "transmart@1",
            "kind": "transmart",
            "enabled": false,
            "extra_field": "preserved"
        });
        let input = json!({
            "translate_services": [baidu, transmart]
        });
        let config: Config = serde_json::from_value(input).unwrap();

        let Service::Baidu(b) = &config.translate_services[0] else {
            panic!("expected baidu");
        };
        assert!(b.enabled);
        assert_eq!(b.config.appid, "baidu-app-id");
        assert_eq!(b.config.secret, "baidu-secret-key");
        assert_eq!(b.extra.get("extra_field"), Some(&json!(100)));

        let Service::Transmart(t) = &config.translate_services[1] else {
            panic!("expected transmart");
        };
        assert!(!t.enabled);
        assert_eq!(t.extra.get("extra_field"), Some(&json!("preserved")));

        let out = serde_json::to_value(&config).unwrap();
        assert_eq!(out["translate_services"][0], baidu);
        assert_eq!(out["translate_services"][1], transmart);
    }

    #[test]
    fn ai_defaults_fill_missing_fields() {
        let service: Service = serde_json::from_value(
            json!({"id": "ai@x", "kind": "ai", "base_url": "u", "model": "m"}),
        )
        .unwrap();
        let Service::Ai(i) = service else {
            panic!("not ai");
        };
        assert!(i.enabled);
        assert_eq!(i.config.protocol, "openai_chat");
        assert_eq!(
            i.config.custom_instructions,
            crate::service::ai::DEFAULT_CUSTOM_INSTRUCTIONS
        );
    }

    #[test]
    fn known_service_without_enabled_is_enabled() {
        let service: Service =
            serde_json::from_value(json!({"id": "g", "kind": "google"})).unwrap();
        assert_eq!(service, Service::Google(Instance::new("g")));
    }

    // 旧 pop_button_distance.test.js + 旧 pop_button.rs 的 button_distance 测试。
    #[test]
    fn button_distance_contract() {
        let parse = |v: Value| {
            let mut c: Config =
                serde_json::from_value(json!({"selection": {"button_distance": v}})).unwrap();
            c.normalize();
            c.selection.button_distance
        };
        for v in [0, 4, 10, 19, 20] {
            assert_eq!(parse(json!(v)), v, "整数原样保留");
        }
        assert_eq!(parse(json!(4.0)), 4, "JSON 的 4.0 也是整数");
        for v in [
            json!(null),
            json!(false),
            json!(true),
            json!(""),
            json!("20"),
            json!([]),
            json!([20]),
            json!({}),
            json!(4.5),
            json!(-0.5),
            json!(20.5),
        ] {
            assert_eq!(parse(v.clone()), 10, "{v} 用默认值");
        }
        for v in [json!(-1), json!(-100), json!(-1.0e300)] {
            assert_eq!(parse(v.clone()), 0, "{v}");
        }
        for v in [
            json!(21),
            json!(37),
            json!(1000),
            json!(1.0e300),
            json!(u64::MAX),
        ] {
            assert_eq!(parse(v.clone()), 20, "{v}");
        }
        let missing: Config = serde_json::from_value(json!({"selection": {}})).unwrap();
        assert_eq!(missing.selection.button_distance, 10);
    }

    #[test]
    fn test_new_instance_id_uniqueness() {
        let id1 = new_instance_id("google");
        let id2 = new_instance_id("google");
        assert_ne!(id1, id2);
        assert!(id1.starts_with("google@"));
        assert!(id2.starts_with("google@"));
    }
}
