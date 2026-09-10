import Database from 'tauri-plugin-sql-api';
import { isWord } from './wordbook_format.js';

// 生词本用自己的库，不和 history.db 共用 —— 批次 9 删 History 页时不牵连。
// 表结构与取舍见 docs/整合方案.md §4.1（一张表 + 一个 detail JSON 列）。
let dbPromise = null;

export function getDB() {
    if (!dbPromise) {
        dbPromise = Database.load('sqlite:wordbook.db')
            .then(async (db) => {
                await db.execute(`CREATE TABLE IF NOT EXISTS entries (
                    id          INTEGER PRIMARY KEY,
                    type        TEXT NOT NULL,
                    text        TEXT NOT NULL,
                    translation TEXT,
                    detail      TEXT,
                    source_id   INTEGER,
                    created_at  INTEGER,
                    deleted     INTEGER DEFAULT 0
                )`);
                await db.execute('CREATE INDEX IF NOT EXISTS idx_entries_type ON entries(type, deleted)');
                return db;
            })
            .catch((e) => {
                // 失败的 promise 缓存下来就再也建不上库了，下次调用重试。
                dbPromise = null;
                throw e;
            });
    }
    return dbPromise;
}

// detail 传对象，这里负责序列化；长句的 AI 结果由批次 8 回填。
export async function addEntry({ text, translation = '', detail = null, sourceId = null, type = null }) {
    const db = await getDB();
    const result = await db.execute(
        'INSERT INTO entries (type, text, translation, detail, source_id, created_at) VALUES ($1, $2, $3, $4, $5, $6)',
        [
            type ?? (isWord(text) ? 'word' : 'sentence'),
            text,
            translation,
            detail && JSON.stringify(detail),
            sourceId,
            Date.now(),
        ]
    );
    return result.lastInsertId;
}
