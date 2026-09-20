//! 翻译调度（design §2.4）：一次划词 = 一个 `query_id`，冻结配置快照，后台检测语种，每个启用的服务一个线程。
//!
//! 两种生命周期分开（旧规范 translation-reliability）：
//! - `query_id` 只保护当前面板：新划词、新截图、取消都推进它，旧结果不再送给界面；
//! - 收藏：点过收藏的那次划词，晚到的结果照样更新自己那条生词本记录，不管面板是否已经换了词。
//!
//! 这里不碰 Slint：结果通过回调在服务线程上交出去，界面层自己 `invoke_from_event_loop` 切回 UI 线程，
//! 并且在 UI 线程上**再比一次** `query_id`（回调发出后到 UI 线程处理之间可能又开了新查询）。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use serde_json::Value;

use crate::error::Error;
use crate::logic::cache::{self, Lru};
use crate::logic::config::{self, Service};
use crate::logic::lang_detect;
use crate::logic::result::{self, EntryDisplay, Kind};
use crate::logic::saved_entry::{Item, SavedEntry, Snapshot};
use crate::logic::wordbook;
use crate::service::{ai, baidu, bing, deepl, detect, google, transmart};

static CURRENT: AtomicU64 = AtomicU64::new(0);
static CACHE: Mutex<Lru> = Mutex::new(Lru::new());

/// 当前面板的 `query_id`。
pub fn current() -> u64 {
    CURRENT.load(Ordering::SeqCst)
}

/// 推进 `query_id`，让在途的结果全部作废（取消、新截图开始时调；`start` 自己也调）。返回新的 id。
pub fn invalidate() -> u64 {
    CURRENT.fetch_add(1, Ordering::SeqCst) + 1
}

/// 结果浮窗的一行。
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub service_id: String,
    /// `"google"` / `"ai"`：界面据此选图标和默认名。
    pub kind: &'static str,
    /// 用户起的名字，空 = 用默认名（界面用 @tr 翻译）。
    pub label: String,
}

/// 一行的结果，已经算好展示投影。
#[derive(Clone, Debug, PartialEq)]
pub struct Shown {
    pub display: EntryDisplay,
    /// 完整纯文本：复制、朗读用。
    pub text: String,
}

#[derive(Debug)]
pub enum Update {
    /// 检测到的语种码（徽标用）；检测失败是 `None`，不显示伪造的徽标。先于所有 `Done` 送达。
    Detected(Option<&'static str>),
    /// 第 `row` 行出结果或出错（超时、连不上都会到这里，不会一直 loading）。
    Done {
        row: usize,
        result: Result<Box<Shown>, Error>,
    },
}

/// 请求开始时冻结的一个服务。
#[derive(Clone)]
pub(crate) enum Request {
    Google(google::Config),
    Bing(bing::Config),
    Deepl(deepl::Config),
    Baidu(baidu::Config),
    Transmart,
    Ai(ai::Effective),
}

struct Job {
    service_id: String,
    request: Request,
    /// 缓存身份里的配置快照（含 key，只在内存）。
    snapshot: Value,
}

type Callback = Arc<dyn Fn(u64, Update) + Send + Sync>;
type OnSaved = Arc<dyn Fn(bool) + Send + Sync>;

struct Saved {
    entry: SavedEntry,
    on_saved: Option<OnSaved>,
}

/// `start` 的返回值：界面立刻按 `rows` 画出 loading 行，之后用它收藏。
pub struct Query {
    pub id: u64,
    /// 预处理后的原文（只去首尾空白，正文换行、公式、标识符原样保留）。
    pub text: String,
    pub rows: Vec<Row>,
    saved: Arc<Mutex<Saved>>,
}

impl Query {
    /// 收藏。`row` 是点了哪一行的星（优先存那一行的结果），顶栏收藏传 `None`。
    /// 之后这次划词的晚到结果继续更新同一条。`on_saved` 在生词本线程上调用，每写一次调一次。
    pub fn save(&self, row: Option<usize>, on_saved: impl Fn(bool) + Send + Sync + 'static) {
        if self.text.is_empty() {
            return;
        }
        let mut saved = self.saved.lock().unwrap_or_else(PoisonError::into_inner);
        saved.on_saved = Some(Arc::new(on_saved));
        let snapshot = saved.entry.save(row);
        persist(self.id, snapshot, saved.on_saved.clone());
    }
}

/// 写库在锁里排队：锁保证快照按「结果到达的顺序」进入生词本线程的 FIFO。
fn persist(handle: u64, snapshot: Snapshot, on_saved: Option<OnSaved>) {
    wordbook::save(handle, snapshot, move |result| {
        if let Some(cb) = on_saved {
            cb(result.is_ok());
        }
    });
}

/// 原文预处理：只裁首尾空白。正文换行、公式、标识符、`$text` 这类文字原样进请求、显示和收藏。
pub fn preprocess(raw: &str) -> &str {
    raw.trim()
}

/// 检测失败（或检测出 `auto`）回退源语言，服务照常请求，不伪造徽标。
pub fn detection_with_fallback(
    detected: Option<&'static str>,
    from: &str,
) -> (String, Option<&'static str>) {
    match detected {
        Some(code) if code != "auto" && !code.is_empty() => (code.to_owned(), Some(code)),
        _ => (from.to_owned(), None),
    }
}

fn detect_language(engine: &str, text: &str) -> Option<&'static str> {
    detect_language_with(engine, text, lang_detect::detect, detect::detect)
}

fn detect_language_with<FLocal, FOnline>(
    engine: &str,
    text: &str,
    local: FLocal,
    online: FOnline,
) -> Option<&'static str>
where
    FLocal: FnOnce(&str) -> Option<&'static str>,
    FOnline: FnOnce(&str, &str) -> Option<&'static str>,
{
    match engine {
        "niutrans" | "baidu" | "google" => online(engine, text),
        _ => local(text),
    }
}

fn job(service: &Service) -> Option<(Row, Job)> {
    let (row, request) = match service {
        Service::Google(i) if i.enabled => (
            Row {
                service_id: i.id.clone(),
                kind: "google",
                label: i.label.clone(),
            },
            Request::Google(i.config.clone()),
        ),
        Service::Bing(i) if i.enabled => (
            Row {
                service_id: i.id.clone(),
                kind: "bing",
                label: i.label.clone(),
            },
            Request::Bing(i.config.clone()),
        ),
        Service::Deepl(i) if i.enabled => (
            Row {
                service_id: i.id.clone(),
                kind: "deepl",
                label: i.label.clone(),
            },
            Request::Deepl(i.config.clone()),
        ),
        Service::Baidu(i) if i.enabled => (
            Row {
                service_id: i.id.clone(),
                kind: "baidu",
                label: i.label.clone(),
            },
            Request::Baidu(i.config.clone()),
        ),
        Service::Transmart(i) if i.enabled => (
            Row {
                service_id: i.id.clone(),
                kind: "transmart",
                label: i.label.clone(),
            },
            Request::Transmart,
        ),
        Service::Ai(i) if i.enabled => (
            Row {
                service_id: i.id.clone(),
                kind: "ai",
                label: i.label.clone(),
            },
            Request::Ai(i.config.effective()),
        ),
        _ => return None,
    };
    let snapshot = match &request {
        Request::Google(c) => serde_json::to_value(c),
        Request::Bing(c) => serde_json::to_value(c),
        Request::Deepl(c) => serde_json::to_value(c),
        Request::Baidu(c) => serde_json::to_value(c),
        Request::Transmart => Ok(Value::Null),
        Request::Ai(e) => serde_json::to_value(e),
    }
    .unwrap_or(Value::Null);
    Some((
        row.clone(),
        Job {
            service_id: row.service_id,
            request,
            snapshot,
        },
    ))
}

/// 开始翻译。在 UI 线程调用：只拷配置、起一个后台线程，不阻塞。
/// `on_update(query_id, update)` 在后台线程上调用；已经不是当前查询的结果不会再调。
/// 原文为空（截图识别还没出字）时只推进 `query_id`、返回空行，不发请求。
pub fn start(raw: &str, on_update: impl Fn(u64, Update) + Send + Sync + 'static) -> Query {
    let id = invalidate();
    let text = preprocess(raw).to_owned();
    let config = config::snapshot();
    let (rows, jobs): (Vec<Row>, Vec<Job>) =
        config.translate_services.iter().filter_map(job).unzip();
    let items = rows
        .iter()
        .map(|r| Item {
            service_id: r.service_id.clone(),
            is_ai: r.kind == "ai",
            result: None,
        })
        .collect();
    let saved = Arc::new(Mutex::new(Saved {
        entry: SavedEntry::new(&text, items),
        on_saved: None,
    }));
    let query = Query {
        id,
        text: text.clone(),
        rows,
        saved: Arc::clone(&saved),
    };
    if text.is_empty() || jobs.is_empty() {
        return query;
    }
    log::info!(
        "Translate: query {id} started, {} services, {} chars",
        jobs.len(),
        text.chars().count()
    );
    let translate_cfg = config.translate.clone();
    let on_update: Callback = Arc::new(on_update);
    let spawned = thread::Builder::new()
        .name("translate".into())
        .spawn(move || dispatch(id, text, translate_cfg, jobs, saved, on_update));
    if let Err(e) = spawned {
        log::error!("Translate: cannot start query thread: {e}");
    }
    query
}

/// 后台线程：检测语种（local 走本地 lingua；niutrans / baidu / google 走在线引擎），再给每个服务起一个线程。
fn dispatch(
    id: u64,
    text: String,
    translate_cfg: config::Translate,
    jobs: Vec<Job>,
    saved: Arc<Mutex<Saved>>,
    on_update: Callback,
) {
    let (detected, badge) = detection_with_fallback(
        detect_language(&translate_cfg.detect_engine, &text),
        &translate_cfg.source,
    );
    let requested = saved
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry
        .requested();
    if id != current() && !requested {
        return;
    }
    if id == current() {
        on_update(id, Update::Detected(badge));
    }
    let text = Arc::new(text);
    for (row, job) in jobs.into_iter().enumerate() {
        let ctx = Ctx {
            id,
            row,
            text: Arc::clone(&text),
            from: translate_cfg.source.clone(),
            to: translate_cfg.target.clone(),
            detected: detected.clone(),
            saved: Arc::clone(&saved),
            on_update: Arc::clone(&on_update),
        };
        let fallback = Arc::clone(&on_update);
        if let Err(e) = thread::Builder::new()
            .name("translate-service".into())
            .spawn(move || run_one(ctx, job))
        {
            log::error!("Translate: cannot start service thread: {e}");
            fallback(
                id,
                Update::Done {
                    row,
                    result: Err(e.into()),
                },
            );
        }
    }
}

struct Ctx {
    id: u64,
    row: usize,
    text: Arc<String>,
    from: String,
    to: String,
    detected: String,
    saved: Arc<Mutex<Saved>>,
    on_update: Callback,
}

fn run_one(ctx: Ctx, job: Job) {
    let kind = match job.request {
        Request::Google(_) => "google",
        Request::Bing(_) => "bing",
        Request::Deepl(_) => "deepl",
        Request::Baidu(_) => "baidu",
        Request::Transmart => "transmart",
        Request::Ai(_) => "ai",
    };
    let key = cache::key(
        &ctx.text,
        &ctx.from,
        &ctx.to,
        &job.service_id,
        &job.snapshot,
        &ctx.detected,
    );
    let cached = CACHE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&key);
    let result = if let Some(value) = cached {
        log::info!("Translate: cache hit ({kind})");
        Ok(value)
    } else {
        let result = call(&job.request, &ctx.text, &ctx.from, &ctx.to, &ctx.detected);
        if let Ok(value) = &result
            && !result::result_text(value).is_empty()
        {
            CACHE
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .put(key, value.clone());
        }
        result
    };
    if let Err(e) = &result {
        // 只记分类：错误里本来就不带响应体、地址和 key。
        log::warn!("Translate: {kind} failed: {e}");
    }
    {
        let mut saved = ctx.saved.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(snapshot) = saved.entry.patch(ctx.row, result.as_ref().ok().cloned()) {
            persist(ctx.id, snapshot, saved.on_saved.clone());
        }
    }
    if ctx.id == current() {
        let shown = result.map(|value| {
            let text = result::result_text(&value);
            let display = result::entry_display(Some(&value), &text);
            Box::new(Shown { display, text })
        });
        (ctx.on_update)(
            ctx.id,
            Update::Done {
                row: ctx.row,
                result: shown,
            },
        );
    }
}

/// 一个 `match` 就是服务注册表（design §4.3）：加服务 = 加一个文件 + 这里加一行。
pub(crate) fn call(
    request: &Request,
    text: &str,
    from: &str,
    to: &str,
    detected: &str,
) -> Result<Value, Error> {
    match request {
        Request::Google(c) => google::translate(text, from, to, c),
        Request::Bing(c) => bing::translate(text, from, to, c),
        Request::Deepl(c) => deepl::translate(text, from, to, c),
        Request::Baidu(c) => baidu::translate(text, from, to, c),
        Request::Transmart => transmart::translate(text, from, to),
        Request::Ai(e) => {
            let kind = Kind::of(text);
            let raw = ai::translate(text, from, to, detected, kind.as_str(), e)?;
            Ok(result::dictionary_result(&raw, kind))
        }
    }
}

fn test_sample(target: &str) -> (&'static str, &'static str) {
    if target.starts_with("zh") {
        ("Hello world", "en")
    } else {
        ("你好世界", "zh_cn")
    }
}

/// 用给定配置真发一次翻译，返回译文。设置窗口的「测试连接」用。
/// 耗时由调用方计：失败时也要显示耗时，计时只能有一处，否则两个数对不上。
pub fn test_google(config: &google::Config) -> Result<String, Error> {
    test_request(&Request::Google(config.clone()))
}

/// 见 [`test_google`]。
pub fn test_bing(config: &bing::Config) -> Result<String, Error> {
    test_request(&Request::Bing(config.clone()))
}

/// 见 [`test_google`]。
pub fn test_deepl(config: &deepl::Config) -> Result<String, Error> {
    test_request(&Request::Deepl(config.clone()))
}

/// 见 [`test_google`]。
pub fn test_baidu(config: &baidu::Config) -> Result<String, Error> {
    test_request(&Request::Baidu(config.clone()))
}

/// 见 [`test_google`]。
pub fn test_transmart() -> Result<String, Error> {
    test_request(&Request::Transmart)
}

/// 见 [`test_google`]。
pub fn test_ai(config: &ai::Config) -> Result<String, Error> {
    test_request(&Request::Ai(config.effective()))
}

fn test_request(request: &Request) -> Result<String, Error> {
    let name = match request {
        Request::Google(_) => "google",
        Request::Bing(_) => "bing",
        Request::Deepl(_) => "deepl",
        Request::Baidu(_) => "baidu",
        Request::Transmart => "transmart",
        Request::Ai(_) => "ai",
    };
    let target = config::snapshot().translate.target;
    let (text, detected) = test_sample(&target);
    // 只记分类：错误里本来就不带响应体、地址和 key。
    let value = call(request, text, "auto", &target, detected)
        .inspect_err(|e| log::warn!("Translate: {name} test failed: {e}"))?;
    Ok(result::result_text(&value))
}

#[cfg(test)]
mod tests {
    use super::*;

    // 旧 detection_fallback.test.js
    #[test]
    fn detection_falls_back_to_the_source_language() {
        assert_eq!(
            detection_with_fallback(Some("ja"), "en"),
            ("ja".into(), Some("ja"))
        );
        for from in ["auto", "en"] {
            assert_eq!(detection_with_fallback(None, from), (from.into(), None));
        }
        for bad in [Some(""), Some("auto"), None] {
            assert_eq!(detection_with_fallback(bad, "auto"), ("auto".into(), None));
        }
    }

    #[test]
    fn query_ids_only_move_forward() {
        let a = invalidate();
        let b = invalidate();
        assert!(b > a);
        assert!(current() >= b, "别的测试也可能同时推进");
    }

    #[test]
    fn only_enabled_known_translate_services_become_rows() {
        let services: Vec<Service> = serde_json::from_value(serde_json::json!([
            {"id": "google", "kind": "google"},
            {"id": "g2", "kind": "google", "enabled": false},
            {"id": "bing@1", "kind": "bing", "label": "BingTr"},
            {"id": "deepl@1", "kind": "deepl", "enabled": false},
            {"id": "deepl@2", "kind": "deepl", "label": "DeepLTr"},
            {"id": "baidu@1", "kind": "baidu", "label": "BaiduTr", "appid": "id", "secret": "sec"},
            {"id": "transmart@1", "kind": "transmart", "label": "TransmartTr"},
            {"id": "ai@x", "kind": "ai", "label": "DS", "base_url": "u", "model": "m", "api_key": "k"},
            {"id": "wechat", "kind": "wechat"},
            {"id": "x", "kind": "future"},
        ]))
        .unwrap();
        let rows: Vec<Row> = services.iter().filter_map(job).map(|(r, _)| r).collect();
        assert_eq!(
            rows.iter()
                .map(|r| (r.service_id.as_str(), r.kind, r.label.as_str()))
                .collect::<Vec<_>>(),
            [
                ("google", "google", ""),
                ("bing@1", "bing", "BingTr"),
                ("deepl@2", "deepl", "DeepLTr"),
                ("baidu@1", "baidu", "BaiduTr"),
                ("transmart@1", "transmart", "TransmartTr"),
                ("ai@x", "ai", "DS")
            ]
        );
    }

    // 旧 text_preprocess.test.js：只裁首尾空白，正文原样（公式、换行、标识符、占位符形状的文字）。
    #[test]
    fn preprocessing_only_trims() {
        for body in [
            "10 - 5 = 5",
            "inter-\nnational",
            "parseUserData",
            "user_id",
            "// comment",
            "# heading",
            "first\n\nsecond",
            "$text $to $&",
        ] {
            assert_eq!(preprocess(&format!("  {body}  ")), body);
        }
    }

    #[test]
    fn empty_text_only_moves_the_query_id() {
        let before = current();
        let q = start("   ", |_, _| panic!("no updates for empty text"));
        assert!(q.id > before);
        assert_eq!(q.text, "");
        q.save(None, |_| panic!("empty text is never saved"));
    }

    #[test]
    fn test_ai_fails_fast_on_missing_settings() {
        let config = ai::Config::default();
        let res = test_ai(&config);
        assert!(res.is_err());
    }

    #[test]
    fn test_bing_fails_fast_on_missing_settings() {
        let config = bing::Config {
            mode: "api".into(),
            ..bing::Config::default()
        };
        let res = test_bing(&config);
        assert!(matches!(res, Err(Error::NotConfigured("auth_key"))));
    }

    #[test]
    fn test_deepl_fails_fast_on_missing_settings() {
        let config = deepl::Config {
            mode: "api".into(),
            ..deepl::Config::default()
        };
        let res = test_deepl(&config);
        assert!(matches!(res, Err(Error::NotConfigured("auth_key"))));

        let config_x = deepl::Config {
            mode: "deeplx".into(),
            ..deepl::Config::default()
        };
        let res_x = test_deepl(&config_x);
        assert!(matches!(res_x, Err(Error::NotConfigured("custom_url"))));
    }

    #[test]
    fn test_baidu_fails_fast_on_missing_settings() {
        let config = baidu::Config {
            appid: "".into(),
            secret: "secret".into(),
        };
        let res = test_baidu(&config);
        assert!(matches!(res, Err(Error::NotConfigured("appid"))));

        let config_secret = baidu::Config {
            appid: "appid".into(),
            secret: "  ".into(),
        };
        let res_secret = test_baidu(&config_secret);
        assert!(matches!(res_secret, Err(Error::NotConfigured("secret"))));
    }

    #[test]
    fn detect_language_dispatches_per_engine() {
        let mut local_called = false;
        let mut online_called = false;
        let res = detect_language_with(
            "local",
            "hello",
            |_| {
                local_called = true;
                Some("en")
            },
            |_, _| {
                online_called = true;
                None
            },
        );
        assert_eq!(res, Some("en"));
        assert!(local_called);
        assert!(!online_called);

        for engine in ["niutrans", "baidu", "google"] {
            let mut dispatched_engine = String::new();
            let res = detect_language_with(
                engine,
                "hello",
                |_| panic!("should not call local for {engine}"),
                |eng, _| {
                    dispatched_engine.push_str(eng);
                    Some("en")
                },
            );
            assert_eq!(dispatched_engine, engine);
            assert_eq!(res, Some("en"));
        }

        for unknown in ["yandex", "tencent", "custom", ""] {
            let mut local_called = false;
            let mut online_called = false;
            let res = detect_language_with(
                unknown,
                "hello",
                |_| {
                    local_called = true;
                    Some("en")
                },
                |_, _| {
                    online_called = true;
                    None
                },
            );
            assert_eq!(res, Some("en"));
            assert!(
                local_called,
                "unknown engine {unknown} should fall back to local"
            );
            assert!(!online_called);
        }
    }

    #[test]
    fn test_sample_selection() {
        assert_eq!(test_sample("zh_cn"), ("Hello world", "en"));
        assert_eq!(test_sample("zh_tw"), ("Hello world", "en"));
        assert_eq!(test_sample("en"), ("你好世界", "zh_cn"));
        assert_eq!(test_sample("ja"), ("你好世界", "zh_cn"));
        assert_eq!(test_sample("ko"), ("你好世界", "zh_cn"));
    }

    #[test]
    fn default_ai_custom_instructions_in_po_matches_constant() {
        let po_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("ui/i18n/zh_CN/LC_MESSAGES/tilex.po");
        let po_content = std::fs::read_to_string(&po_path).unwrap();

        fn unescape_po(s: &str) -> String {
            let mut out = String::new();
            let mut chars = s.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    match chars.next() {
                        Some('"') => out.push('"'),
                        Some('\\') => out.push('\\'),
                        Some('n') => out.push('\n'),
                        Some('t') => out.push('\t'),
                        Some(other) => {
                            out.push('\\');
                            out.push(other);
                        }
                        None => out.push('\\'),
                    }
                } else {
                    out.push(c);
                }
            }
            out
        }

        fn parse_po_entries(text: &str) -> Vec<(String, String)> {
            let mut entries = Vec::new();
            let mut cur_id: Option<String> = None;
            let mut cur_str = String::new();
            let mut in_msgstr = false;

            let extract_str = |line: &str| -> Option<String> {
                let start = line.find('"')?;
                let end = line.rfind('"')?;
                if start < end {
                    Some(unescape_po(&line[start + 1..end]))
                } else {
                    None
                }
            };

            for line in text.lines().map(str::trim) {
                if let Some(rest) = line.strip_prefix("msgid ") {
                    if let Some(prev_id) = cur_id.take() {
                        entries.push((prev_id, std::mem::take(&mut cur_str)));
                    }
                    cur_id = extract_str(rest).or(Some(String::new()));
                    in_msgstr = false;
                } else if let Some(rest) = line.strip_prefix("msgstr ") {
                    cur_str = extract_str(rest).unwrap_or_default();
                    in_msgstr = true;
                } else if line.starts_with('"')
                    && let Some(part) = extract_str(line)
                {
                    if in_msgstr {
                        cur_str.push_str(&part);
                    } else if let Some(id) = cur_id.as_mut() {
                        id.push_str(&part);
                    }
                }
            }
            if let Some(id) = cur_id {
                entries.push((id, cur_str));
            }
            entries
        }

        let entries = parse_po_entries(&po_content);
        let found = entries
            .iter()
            .find(|(_, s)| s == ai::DEFAULT_CUSTOM_INSTRUCTIONS);
        assert!(
            found.is_some(),
            "tilex.po must contain a msgstr exactly equal to DEFAULT_CUSTOM_INSTRUCTIONS"
        );
        let (msgid, msgstr) = found.unwrap();
        assert_eq!(msgstr, ai::DEFAULT_CUSTOM_INSTRUCTIONS);
        assert_eq!(
            msgid,
            "Accurately, naturally, and concisely convey the original meaning in the selected target language. Use context to understand polysemous words, and prioritize standard terminology for specialized content. For words and phrases, highlight common definitions and collocations; for sentences, preserve the original meaning and tone, and break down complex sentences into core structures, modifiers, and key terms. Provide brief examples or explanations when necessary, and avoid irrelevant elaborations."
        );
    }
}
