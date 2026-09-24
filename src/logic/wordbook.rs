//! 生词本（design §2.3）：`<数据目录>/wordbook.db`，一个生词本线程独占 SQLite 连接，按 channel 顺序执行。
//! FIFO 天然保证同一条目「新增 → 晚到更新」有序，不需要单独的写队列。
//! 打开失败不缓存失败状态：下次操作重试打开（旧规范「失败的 dbPromise 不能缓存」）。
//!
//! 列表一次取全（几千条摘要几百 KB），筛选和搜索是内存里的纯函数 [`visible`]，
//! 跟旧 `wordbook_selection.js` 一一对应：「全选当前可见」和「删除后顺延」都按同一份可见顺序算。
//! `detail` 只在查列表时用来算摘要，不留在内存里；右卡和导出各自再查。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params, params_from_iter};
use serde_json::Value;

use crate::error::Error;
use crate::logic::result::{Kind, entry_display};
use crate::logic::saved_entry::Snapshot;

pub const FILE: &str = "wordbook.db";

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS entries (
  id          INTEGER PRIMARY KEY,
  kind        TEXT NOT NULL CHECK (kind IN ('word','sentence')),
  text        TEXT NOT NULL,
  translation TEXT NOT NULL DEFAULT '',
  detail      TEXT,
  source_id   INTEGER REFERENCES entries(id),
  service     TEXT NOT NULL DEFAULT '',
  created_at  INTEGER NOT NULL,
  updated_at  INTEGER NOT NULL,
  deleted     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_entries_kind ON entries(kind, deleted);
PRAGMA user_version = 1;
";

/// 列表只拿摘要，点开再取详情。
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub id: i64,
    pub kind: Kind,
    pub text: String,
    /// 列表次行：单词取释义摘要，没有就退回译文（旧版 `wordSummary(detail) || translation`）。
    pub preview: String,
    /// 搜索匹配的那串：原文 + 译文 + 释义摘要，已经小写。
    search: String,
}

/// 一条完整记录：右卡详情和导出共用。
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: i64,
    pub kind: Kind,
    pub text: String,
    pub translation: String,
    pub detail: Option<Value>,
    pub service: String,
    pub created_at: i64,
}

/// 列表一行只有一行的高度（50px 固定行高），所以摘要里的换行要压成空格。
/// `overflow: elide` 只管一行放不下的情况，文本里真有 `\n` 时 Slint 照样换行，行会串到下一条上面去。
fn one_line(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for c in s.trim().chars() {
        if c.is_whitespace() {
            if !in_ws {
                out.push(' ');
                in_ws = true;
            }
        } else {
            out.push(c);
            in_ws = false;
        }
    }
    out
}

/// 单词的 detail 就是浮窗存下的词典结果，摘要 = 各词性的释义拼一行（旧 `wordSummary`）。
fn summarize(detail: Option<&Value>, translation: &str) -> String {
    entry_display(detail, translation)
        .explanations
        .iter()
        .map(|e| e.explains.join(", "))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("; ")
}

/// 当前可见顺序：先按 kind 筛，再按关键词搜原文 / 译文 / 释义三处（旧 `matchEntry`）。
/// 关键词自己 trim + 小写，调用方不用预处理。
pub fn visible<'a>(entries: &'a [Summary], keyword: &str, kind: Option<Kind>) -> Vec<&'a Summary> {
    let kw = keyword.trim().to_lowercase();
    entries
        .iter()
        .filter(|e| kind.is_none_or(|k| k == e.kind))
        .filter(|e| kw.is_empty() || e.search.contains(&kw))
        .collect()
}

/// 在 `text` 中查找所有大小写不敏感匹配 `keyword` 的字节范围 `Range<usize>`。
/// 保证返回的每个 Range 都在 UTF-8 字符边界上，支持多语言（包括德语、土耳其语、中日韩等）。
pub fn find_keyword_ranges(text: &str, keyword: &str) -> Vec<std::ops::Range<usize>> {
    let trimmed_kw = keyword.trim();
    if trimmed_kw.is_empty() || text.is_empty() {
        return Vec::new();
    }
    let kw_chars: Vec<char> = trimmed_kw.chars().flat_map(|c| c.to_lowercase()).collect();
    if kw_chars.is_empty() {
        return Vec::new();
    }

    let text_chars: Vec<(usize, char)> = text.char_indices().collect();
    let n = text_chars.len();
    let mut ranges = Vec::new();
    let mut i = 0;

    while i < n {
        let mut buf: Vec<char> = Vec::new();
        let mut matched = false;
        let mut match_end_char_idx = i;

        for (j, &(_, ch)) in text_chars.iter().enumerate().skip(i) {
            for lc in ch.to_lowercase() {
                buf.push(lc);
            }
            if buf.len() > kw_chars.len() || !kw_chars.starts_with(&buf) {
                break;
            }
            if buf == kw_chars {
                matched = true;
                match_end_char_idx = j;
                break;
            }
        }

        if matched {
            let start_byte = text_chars[i].0;
            let end_byte = if match_end_char_idx + 1 < n {
                text_chars[match_end_char_idx + 1].0
            } else {
                text.len()
            };
            ranges.push(start_byte..end_byte);
            i = match_end_char_idx + 1;
        } else {
            i += 1;
        }
    }

    ranges
}

/// 转义 Markdown / CommonMark 特殊符号，避免高亮时被 Slint 解析为排版标记。
pub fn escape_markdown(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' | '*' | '_' | '[' | ']' | '<' | '>' | '`' | '~' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// 根据关键词对 `text` 进行 Markdown 格式高亮：
/// 匹配部分用 `<font color="{accent_color}">**{escaped_matched}**</font>` 包裹。
/// 未匹配部分及特殊字符均进行 Markdown 转义。
pub fn highlight_markdown(text: &str, keyword: &str, accent_color: &str) -> String {
    let ranges = find_keyword_ranges(text, keyword);
    if ranges.is_empty() {
        return escape_markdown(text);
    }
    let mut out = String::with_capacity(text.len() + ranges.len() * 40);
    let mut last = 0;
    for r in ranges {
        if r.start > last {
            out.push_str(&escape_markdown(&text[last..r.start]));
        }
        let matched = escape_markdown(&text[r.start..r.end]);
        out.push_str("<font color=\"");
        out.push_str(accent_color);
        out.push_str("\">**");
        out.push_str(&matched);
        out.push_str("**</font>");
        last = r.end;
    }
    if last < text.len() {
        out.push_str(&escape_markdown(&text[last..]));
    }
    out
}

/// 删除之后预览停在哪条：还在就不动；被删了就往下找第一条没被删的，没有再往上找，都没有就空。
/// `visible` 是**删除前**的可见顺序；选中项不在里面（或没选）时从首项算起，和旧版 `?? list[0]` 一致。
pub fn next_selected(visible: &[i64], removed: &[i64], selected: Option<i64>) -> Option<i64> {
    let gone = |id: &i64| removed.contains(id);
    let index = selected
        .and_then(|id| visible.iter().position(|v| *v == id))
        .unwrap_or(0);
    let &current = visible.get(index)?;
    if !gone(&current) {
        return Some(current);
    }
    visible[index + 1..]
        .iter()
        .find(|id| !gone(id))
        .or_else(|| visible[..index].iter().rev().find(|id| !gone(id)))
        .copied()
}

/// 一个生词本文件和它的连接。线程外（测试）也能直接用。
pub struct Db {
    path: PathBuf,
    conn: Option<Connection>,
    /// 收藏句柄（一次划词的 query_id）→ 它插入的那一行。同一次划词第二次起只更新这一行。
    /// ponytail: 只增不减，每次收藏一个整数，进程生命周期内可以忽略。
    rows: HashMap<u64, i64>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

impl Db {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            conn: None,
            rows: HashMap::new(),
        }
    }

    fn conn(&mut self) -> Result<&Connection, Error> {
        if self.conn.is_none() {
            if let Some(dir) = self.path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let conn = Connection::open(&self.path)?;
            conn.execute_batch(SCHEMA)?;
            self.conn = Some(conn);
        }
        self.conn
            .as_ref()
            .ok_or_else(|| Error::Platform("wordbook connection missing".into()))
    }

    /// 收藏写库：这个句柄第一次写就插入，之后只更新同一行。
    /// 更新带 `deleted = 0`：用户在生词本里删掉之后，晚到的结果不会让它复活，也不会重新插入。
    pub fn save(&mut self, handle: u64, entry: &Snapshot) -> Result<i64, Error> {
        let now = now_ms();
        let detail = entry.detail.as_ref().map(ToString::to_string);
        if let Some(&id) = self.rows.get(&handle) {
            self.conn()?.execute(
                "UPDATE entries SET translation = ?1, detail = ?2, service = ?3, updated_at = ?4 \
                 WHERE id = ?5 AND deleted = 0",
                params![entry.translation, detail, entry.service, now, id],
            )?;
            return Ok(id);
        }
        let conn = self.conn()?;
        conn.execute(
            "INSERT INTO entries (kind, text, translation, detail, service, created_at, updated_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![
                Kind::of(&entry.text).as_str(),
                entry.text,
                entry.translation,
                detail,
                entry.service,
                now
            ],
        )?;
        let id = conn.last_insert_rowid();
        self.rows.insert(handle, id);
        Ok(id)
    }

    /// 软删除，一条参数化 `UPDATE ... WHERE id IN (...)`。空集合不执行 SQL，重复 id 去重。返回删掉的行数。
    pub fn soft_delete(&mut self, ids: &[i64]) -> Result<usize, Error> {
        let mut unique = ids.to_vec();
        unique.sort_unstable();
        unique.dedup();
        if unique.is_empty() {
            return Ok(0);
        }
        let marks: Vec<String> = (2..unique.len() + 2).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "UPDATE entries SET deleted = 1, updated_at = ?1 WHERE deleted = 0 AND id IN ({})",
            marks.join(", ")
        );
        let now = now_ms();
        let values = std::iter::once(now).chain(unique);
        Ok(self.conn()?.execute(&sql, params_from_iter(values))?)
    }

    /// 未删除的条目，新的在前。
    pub fn list(&mut self) -> Result<Vec<Summary>, Error> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, kind, text, translation, detail FROM entries WHERE deleted = 0 \
             ORDER BY created_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let text: String = row.get(2)?;
            let translation: String = row.get(3)?;
            let detail = parse_detail(row.get(4)?);
            let summary = summarize(detail.as_ref(), &translation);
            Ok(Summary {
                id: row.get(0)?,
                kind: kind_of(&row.get::<_, String>(1)?),
                search: format!("{text} {translation} {summary}").to_lowercase(),
                preview: one_line(if summary.is_empty() {
                    &translation
                } else {
                    &summary
                }),
                text: one_line(&text),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// 一条完整记录，给右卡详情。已删除的取不到。
    pub fn entry(&mut self, id: i64) -> Result<Option<Entry>, Error> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!("{ENTRY_COLUMNS} AND id = ?1"))?;
        let mut rows = stmt.query_map([id], read_entry)?;
        rows.next().transpose().map_err(Into::into)
    }

    /// 全部未删除记录（含 detail），顺序同 [`Db::list`]。导出用。
    pub fn all(&mut self) -> Result<Vec<Entry>, Error> {
        let conn = self.conn()?;
        let mut stmt = conn.prepare(&format!(
            "{ENTRY_COLUMNS} ORDER BY created_at DESC, id DESC"
        ))?;
        let rows = stmt.query_map([], read_entry)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

const ENTRY_COLUMNS: &str = "SELECT id, kind, text, translation, detail, service, created_at \
                             FROM entries WHERE deleted = 0";

fn kind_of(s: &str) -> Kind {
    if s == "sentence" {
        Kind::Sentence
    } else {
        Kind::Word
    }
}

/// 坏掉的 detail 当没有：界面和导出退回纯译文那条路（旧规范「损坏 detail 使用唯一译文」）。
fn parse_detail(raw: Option<String>) -> Option<Value> {
    raw.and_then(|s| serde_json::from_str(&s).ok())
}

fn read_entry(row: &rusqlite::Row) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: row.get(0)?,
        kind: kind_of(&row.get::<_, String>(1)?),
        text: row.get(2)?,
        translation: row.get(3)?,
        detail: parse_detail(row.get(4)?),
        service: row.get(5)?,
        created_at: row.get(6)?,
    })
}

type Job = Box<dyn FnOnce(Result<&mut Db, Error>) + Send>;

static QUEUE: OnceLock<Sender<Job>> = OnceLock::new();

/// 启动时调一次：起生词本线程。数据库文件第一次用时才打开。
pub fn init(dir: &Path) -> Result<(), Error> {
    let (tx, rx) = mpsc::channel::<Job>();
    let mut db = Db::new(dir.join(FILE));
    std::thread::Builder::new()
        .name("wordbook".into())
        .spawn(move || {
            for job in rx {
                job(Ok(&mut db));
            }
        })?;
    QUEUE
        .set(tx)
        .map_err(|_| Error::Platform("wordbook already initialized".into()))
}

/// 排进生词本线程。没初始化或线程已退出时，当场以错误调用 `job`，回调不会丢。
fn submit(job: Job) {
    let Some(tx) = QUEUE.get() else {
        job(Err(Error::Platform("wordbook not initialized".into())));
        return;
    };
    if let Err(mpsc::SendError(job)) = tx.send(job) {
        log::error!("Wordbook: worker thread is gone");
        job(Err(Error::Platform("wordbook thread stopped".into())));
    }
}

type Changed = Box<dyn Fn() + Send>;

/// 「生词本内容变了」的观察者。生词本页打开时注册、关闭时清掉，所以只需要一个。
static CHANGED: Mutex<Option<Changed>> = Mutex::new(None);

/// 注册 / 清掉变更通知。回调**在生词本线程上**调用，实现里要自己切回 UI 线程。
/// 用途：浮窗收藏之后，已经打开的生词本页自己刷新出新条目。
pub fn set_on_changed(cb: Option<Changed>) {
    *CHANGED.lock().unwrap_or_else(PoisonError::into_inner) = cb;
}

fn notify_changed() {
    if let Some(cb) = CHANGED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        cb();
    }
}

/// 收藏写库（见 [`Db::save`]）。`done` 在生词本线程上调用。
pub fn save(handle: u64, entry: Snapshot, done: impl FnOnce(Result<i64, Error>) + Send + 'static) {
    submit(Box::new(move |db| {
        let result = db.and_then(|db| db.save(handle, &entry));
        match &result {
            Ok(id) => log::info!("Wordbook: saved entry {id}"),
            Err(e) => log::warn!("Wordbook: save failed: {e}"),
        }
        if result.is_ok() {
            notify_changed();
        }
        done(result);
    }));
}

/// 软删除（见 [`Db::soft_delete`]）。`done` 在生词本线程上调用。
pub fn delete(ids: Vec<i64>, done: impl FnOnce(Result<usize, Error>) + Send + 'static) {
    submit(Box::new(move |db| {
        let result = db.and_then(|db| db.soft_delete(&ids));
        // 只有真删掉了才叫：空集合和「删的都是已删除的」不触发刷新
        if matches!(result, Ok(n) if n > 0) {
            notify_changed();
        }
        done(result)
    }));
}

/// 列表摘要（见 [`Db::list`]）。`done` 在生词本线程上调用。
pub fn list(done: impl FnOnce(Result<Vec<Summary>, Error>) + Send + 'static) {
    submit(Box::new(move |db| done(db.and_then(Db::list))));
}

/// 一条详情（见 [`Db::entry`]）。`done` 在生词本线程上调用。
pub fn entry(id: i64, done: impl FnOnce(Result<Option<Entry>, Error>) + Send + 'static) {
    submit(Box::new(move |db| done(db.and_then(|db| db.entry(id)))));
}

/// 全部记录（见 [`Db::all`]）。`done` 在生词本线程上调用。
pub fn all(done: impl FnOnce(Result<Vec<Entry>, Error>) + Send + 'static) {
    submit(Box::new(move |db| done(db.and_then(Db::all))));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_db(name: &str) -> (PathBuf, Db) {
        let dir =
            std::env::temp_dir().join(format!("tilex-wordbook-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let db = Db::new(dir.join(FILE));
        (dir, db)
    }

    fn snap(text: &str, translation: &str) -> Snapshot {
        Snapshot {
            text: text.into(),
            translation: translation.into(),
            detail: None,
            service: "google".into(),
        }
    }

    fn row(db: &mut Db, id: i64) -> (String, Option<String>, i64) {
        db.conn()
            .unwrap()
            .query_row(
                "SELECT translation, detail, deleted FROM entries WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    }

    #[test]
    fn one_selection_inserts_once_then_updates_its_row() {
        let (dir, mut db) = temp_db("once");
        let id = db.save(7, &snap("translation", "")).unwrap();
        let mut late = snap("translation", "译文");
        late.detail = Some(json!({"explanations": [{"explains": ["译文"]}]}));
        assert_eq!(db.save(7, &late).unwrap(), id);
        assert_eq!(db.list().unwrap().len(), 1);
        let (translation, detail, _) = row(&mut db, id);
        assert_eq!(translation, "译文");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&detail.unwrap()).unwrap(),
            late.detail.unwrap()
        );
        // 另一次划词（同原文、不同目标语言）是另一条
        let other = db.save(8, &snap("translation", "翻訳")).unwrap();
        assert_ne!(other, id);
        assert_eq!(row(&mut db, id).0, "译文");
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn soft_deleted_rows_never_revive() {
        let (dir, mut db) = temp_db("deleted");
        let id = db.save(1, &snap("source", "early")).unwrap();
        assert_eq!(db.soft_delete(&[id, id]).unwrap(), 1);
        assert_eq!(db.save(1, &snap("source", "late AI")).unwrap(), id);
        let (translation, _, deleted) = row(&mut db, id);
        assert_eq!((translation.as_str(), deleted), ("early", 1));
        assert!(db.list().unwrap().is_empty(), "晚到结果不重新插入");
        assert_eq!(db.soft_delete(&[]).unwrap(), 0);
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn list_is_newest_first_with_kinds() {
        let (dir, mut db) = temp_db("list");
        let a = db.save(1, &snap("latency", "延迟")).unwrap();
        let b = db
            .save(2, &snap("Network latency matters.", "网络延迟很重要。"))
            .unwrap();
        let c = db.save(3, &snap("gone", "")).unwrap();
        assert_eq!(db.soft_delete(&[c, 999]).unwrap(), 1);
        let list = db.list().unwrap();
        assert_eq!(list.iter().map(|s| s.id).collect::<Vec<_>>(), [b, a]);
        assert_eq!((list[0].kind, list[1].kind), (Kind::Sentence, Kind::Word));
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn open_failure_is_retried() {
        let (dir, mut db) = temp_db("retry");
        // 路径上放一个目录：打开失败
        std::fs::create_dir_all(dir.join(FILE)).unwrap();
        assert!(db.save(1, &snap("w", "t")).is_err());
        std::fs::remove_dir(dir.join(FILE)).unwrap();
        assert!(
            db.save(1, &snap("w", "t")).is_ok(),
            "失败不缓存，下次重试打开"
        );
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn summary_previews_and_searches_three_places() {
        let (dir, mut db) = temp_db("summary");
        let mut word = snap("open-source", "开源");
        word.detail = Some(json!({
            "kind": "word",
            "explanations": [{"trait": "adj.", "explains": ["开源的", "开放源码"]},
                             {"trait": "v.", "explains": ["开放源码"]}],
        }));
        db.save(1, &word).unwrap();
        db.save(2, &snap("Latency matters.", "延迟很重要。"))
            .unwrap();

        let list = db.list().unwrap();
        // 单词次行是释义摘要；句子没有释义，退回译文
        assert_eq!(list[1].preview, "开源的, 开放源码; 开放源码");
        assert_eq!(list[0].preview, "延迟很重要。");
        // 原文、译文、释义三处都能搜到；大小写无关、两头空白无关
        for kw in ["  OPEN-Source ", "开源", "开放源码"] {
            let hit = visible(&list, kw, None);
            assert_eq!(hit.len(), 1, "{kw} 应只命中单词那条");
            assert_eq!(hit[0].text, "open-source");
        }
        assert!(visible(&list, "没有这个词", None).is_empty());

        assert_eq!(visible(&list, "", Some(Kind::Sentence)).len(), 1);
        assert_eq!(visible(&list, "开源", Some(Kind::Sentence)).len(), 0);
        assert_eq!(visible(&list, "", None).len(), 2);

        // 原文和摘要里的换行压成一个空格：列表行高固定 50px，真换行会串到下一条上面
        db.save(3, &snap("Two\nlines.", "第一行\r\n  第二行  "))
            .unwrap();
        let list = db.list().unwrap();
        assert_eq!(list[0].text, "Two lines.");
        assert_eq!(list[0].preview, "第一行 第二行");
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn entry_reads_detail_and_skips_broken_json() {
        let (dir, mut db) = temp_db("entry");
        let mut good = snap("gemini", "双子座");
        good.detail = Some(json!({"kind": "word", "pronunciations": [{"symbol": "/ˈdʒɛmɪnaɪ/"}]}));
        let id = db.save(1, &good).unwrap();
        let broken = db.save(2, &snap("broken", "坏的")).unwrap();
        db.conn()
            .unwrap()
            .execute(
                "UPDATE entries SET detail = '{oops' WHERE id = ?1",
                [broken],
            )
            .unwrap();

        let e = db.entry(id).unwrap().unwrap();
        assert_eq!(
            (e.id, e.kind, e.service.as_str()),
            (id, Kind::Word, "google")
        );
        assert_eq!(
            e.detail.unwrap()["pronunciations"][0]["symbol"],
            "/ˈdʒɛmɪnaɪ/"
        );
        assert!(
            db.entry(broken).unwrap().unwrap().detail.is_none(),
            "坏 detail 当没有"
        );
        assert!(db.list().unwrap().iter().any(|s| s.preview == "坏的"));

        assert_eq!(db.all().unwrap().len(), 2);
        db.soft_delete(&[broken]).unwrap();
        assert!(db.entry(broken).unwrap().is_none(), "删掉的取不到");
        assert_eq!(db.all().unwrap().len(), 1);
        assert!(db.entry(999).unwrap().is_none());
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preview_moves_down_then_up_after_delete() {
        let view = [1, 2, 3, 4, 5];
        // 没被删就不动
        assert_eq!(next_selected(&view, &[4], Some(2)), Some(2));
        // 删第三项 → 原第四项
        assert_eq!(next_selected(&view, &[3], Some(3)), Some(4));
        // 批量：跳过整个删除集合
        assert_eq!(next_selected(&view, &[3, 4], Some(3)), Some(5));
        // 删末项 → 往上找
        assert_eq!(next_selected(&view, &[5], Some(5)), Some(4));
        assert_eq!(next_selected(&view, &[4, 5], Some(5)), Some(3));
        // 全删 → 空
        assert_eq!(next_selected(&view, &view, Some(3)), None);
        // 没选过 / 选的项已不在可见列表里：从首项算起
        assert_eq!(next_selected(&view, &[], None), Some(1));
        assert_eq!(next_selected(&view, &[1], Some(99)), Some(2));
        // 空列表
        assert_eq!(next_selected(&[], &[1], Some(1)), None);
    }

    #[test]
    fn user_version_is_set() {
        let (dir, mut db) = temp_db("version");
        let v: i64 = db
            .conn()
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 1);
        drop(db);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn test_find_keyword_ranges_and_highlight() {
        // 大小写不敏感匹配
        let ranges = find_keyword_ranges("KB kb Kb kB", "kb");
        assert_eq!(ranges, vec![0..2, 3..5, 6..8, 9..11]);

        // 空关键词或空文本
        assert!(find_keyword_ranges("hello", "").is_empty());
        assert!(find_keyword_ranges("hello", "   ").is_empty());
        assert!(find_keyword_ranges("", "hello").is_empty());

        // 中文字符
        let cn = "长难句核心句型与主干解析";
        let ranges_cn = find_keyword_ranges(cn, "核心句型");
        assert_eq!(ranges_cn, vec![9..21]);
        assert_eq!(&cn[9..21], "核心句型");

        // 土耳其语特殊字符 İ
        let tr_text = "İstanbul";
        let ranges_tr = find_keyword_ranges(tr_text, "İSTANBUL");
        assert_eq!(ranges_tr, vec![0..9]);

        // Markdown 字符转义
        assert_eq!(
            escape_markdown("a*b_c[d]<e>`f~g\\h"),
            "a\\*b\\_c\\[d\\]\\<e\\>\\`f\\~g\\\\h"
        );

        // 高亮包裹
        let hl = highlight_markdown("Hello World", "world", "#3b82f6");
        assert_eq!(hl, "Hello <font color=\"#3b82f6\">**World**</font>");

        // 包含特殊符号的高亮文本
        let hl_spec = highlight_markdown("Notice: [tag] *bold*", "tag", "#3b82f6");
        assert_eq!(
            hl_spec,
            "Notice: \\[<font color=\"#3b82f6\">**tag**</font>\\] \\*bold\\*"
        );
    }
}
