// Exercise the production export entry with controlled native boundaries.
// The native save dialog / filesystem scope still require desktop acceptance.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as format from './wordbook_format.js';

const source = readFileSync(new URL('./wordbook_export.js', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;
const now = new Date(2026, 8, 12, 16, 30);
const entry = {
    id: 1,
    type: 'sentence',
    text: 'Keep <vector> and user_id.',
    deleted: 0,
    translation: 'SEARCHABLE_FLAT_TEXT',
    detail: JSON.stringify({
        schemaVersion: 1,
        kind: 'sentence',
        translation: '保留 <vector> 和 user_id。',
        examples: [{ text: 'Use [text](url).', translation: '使用字面符号。' }],
        notes: ['不要删除代码字符。'],
    }),
};

function setup({ rows = [entry], path = 'chosen.md', saveError, writeError, selectError, saveWait } = {}) {
    const calls = [];
    const native = {
        '@tauri-apps/api/fs': {
            writeTextFile: async (...args) => {
                calls.push(['write', ...args]);
                if (writeError) throw writeError;
            },
        },
        '@tauri-apps/api/dialog': {
            save: async (options) => {
                calls.push(['save', options]);
                if (saveError) throw saveError;
                if (saveWait) await saveWait;
                return path;
            },
        },
        './wordbook_format.js': { ...format, buildMarkdown: (values) => format.buildMarkdown(values, now) },
        './wordbook.js': {
            getDB: async () => ({
                select: async (query) => {
                    calls.push(['select', query]);
                    assert.equal(query, 'SELECT * FROM entries WHERE deleted=0');
                    if (selectError) throw selectError;
                    return rows.filter((row) => row.deleted === 0);
                },
            }),
        },
    };
    const exports = {};
    new Function('require', 'exports', compiled)((name) => {
        assert.ok(Object.hasOwn(native, name), `Unexpected export dependency: ${name}`);
        return native[name];
    }, exports);
    return { exportMarkdown: exports.exportMarkdown, calls };
}

const cancel = setup({ path: null });
assert.equal(cancel.calls.length, 0, 'importing the module cannot open a dialog or write');
assert.equal(await cancel.exportMarkdown(), null);
assert.deepEqual(
    cancel.calls.map(([operation]) => operation),
    ['select', 'save']
);
assert.deepEqual(cancel.calls[1][1], {
    defaultPath: '我的生词本.md',
    filters: [{ name: 'Markdown', extensions: ['md'] }],
});

let release;
const wait = new Promise((resolve) => {
    release = resolve;
});
const chosen = setup({ rows: [entry, { ...entry, id: 2, text: 'DELETED_TEXT', deleted: 1 }], saveWait: wait });
const original = JSON.stringify(entry);
const pending = chosen.exportMarkdown();
await Promise.resolve();
await Promise.resolve();
assert.deepEqual(
    chosen.calls.map(([operation]) => operation),
    ['select', 'save']
);
release();
assert.equal(await pending, 'chosen.md');
assert.deepEqual(
    chosen.calls.map(([operation]) => operation),
    ['select', 'save', 'write']
);
assert.equal(chosen.calls[2][1], 'chosen.md');
assert.equal(chosen.calls[2][2], format.buildMarkdown([entry], now));
assert.ok(!chosen.calls[2][2].includes('DELETED_TEXT') && !chosen.calls[2][2].includes('SEARCHABLE_FLAT_TEXT'));
assert.equal(JSON.stringify(entry), original, 'export does not mutate or migrate stored rows');

const empty = setup({ rows: [] });
assert.equal(await empty.exportMarkdown(), 'chosen.md');
assert.equal(empty.calls[2][2], format.buildMarkdown([], now));
const corrupt = setup({ rows: [{ ...entry, detail: '{broken', translation: 'RAW_FALLBACK' }] });
await corrupt.exportMarkdown();
assert.ok(corrupt.calls[2][2].includes('RAW&#95;FALLBACK'));

for (const [boundary, operations] of [
    ['selectError', ['select']],
    ['saveError', ['select', 'save']],
    ['writeError', ['select', 'save', 'write']],
]) {
    const error = new Error(boundary);
    const failing = setup({ [boundary]: error });
    await assert.rejects(failing.exportMarkdown(), (received) => received === error);
    assert.deepEqual(
        failing.calls.map(([operation]) => operation),
        operations
    );
}
console.log(
    'wordbook_export: explicit save, cancellation, scope, empty/corrupt data and native-error propagation passed'
);
