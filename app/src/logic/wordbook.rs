//! 生词本（design §2.3）：`<数据目录>/wordbook.db`，一个生词本线程独占 SQLite 连接，按 channel 顺序执行。
//! FIFO 天然保证同一条目「新增 → 晚到更新」有序，不需要单独的写队列。
//! 打开失败不缓存失败状态：下次操作重试打开（旧规范「失败的 dbPromise 不能缓存」）。
//! 页面（列表、详情、批量删除界面、导出）在 B4。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::mpsc::{self, Sender};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, params, params_from_iter};

use crate::error::Error;
use crate::logic::result::Kind;
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

/// 列表只拿摘要，点开再取详情（B4）。
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub id: i64,
    pub kind: Kind,
    pub text: String,
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
            "SELECT id, kind, text FROM entries WHERE deleted = 0 ORDER BY created_at DESC, id DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let kind: String = row.get(1)?;
            Ok(Summary {
                id: row.get(0)?,
                kind: if kind == "sentence" {
                    Kind::Sentence
                } else {
                    Kind::Word
                },
                text: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
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

/// 收藏写库（见 [`Db::save`]）。`done` 在生词本线程上调用。
pub fn save(handle: u64, entry: Snapshot, done: impl FnOnce(Result<i64, Error>) + Send + 'static) {
    submit(Box::new(move |db| {
        let result = db.and_then(|db| db.save(handle, &entry));
        match &result {
            Ok(id) => log::info!("Wordbook: saved entry {id}"),
            Err(e) => log::warn!("Wordbook: save failed: {e}"),
        }
        done(result);
    }));
}

/// 软删除（见 [`Db::soft_delete`]）。`done` 在生词本线程上调用。
pub fn delete(ids: Vec<i64>, done: impl FnOnce(Result<usize, Error>) + Send + 'static) {
    submit(Box::new(move |db| {
        done(db.and_then(|db| db.soft_delete(&ids)))
    }));
}

/// 列表摘要（见 [`Db::list`]）。`done` 在生词本线程上调用。
pub fn list(done: impl FnOnce(Result<Vec<Summary>, Error>) + Send + 'static) {
    submit(Box::new(move |db| done(db.and_then(Db::list))));
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
}
