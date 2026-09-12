import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { createRequire } from 'node:module';
import { webcrypto } from 'node:crypto';
import ts from 'typescript';
import { runOcrRequest, createOcrEventGate } from './ocr_request.js';

// Use the installed Tauri invoke implementation, including its transport envelope.
// The production Screenshot callback is extracted verbatim: mocking invoke or
// duplicating its arguments here would miss collisions with Tauri's callback IDs.
const require = createRequire(import.meta.url);
const { invoke } = require('@tauri-apps/api/tauri');
const screenshotPath = new URL('../window/Screenshot/index.jsx', import.meta.url);
const source = ts.createSourceFile(screenshotPath.pathname, readFileSync(screenshotPath, 'utf8'), ts.ScriptTarget.Latest, true, ts.ScriptKind.JSX);
let publishSource;
function visit(node) {
    if (ts.isPropertyAssignment(node) && node.name.getText(source) === 'publish') publishSource = node.initializer.getText(source);
    ts.forEachChild(node, visit);
}
visit(source);
assert.ok(publishSource, 'Screenshot must wire its production publication callback');
const publish = new Function('invoke', 'requestId', `return (${publishSource});`)(invoke, 7);

const originalWindow = globalThis.window;
const events = [];
const displayed = [];
const cleaned = [];
let active = true;
let failPublication = false;
const gate = createOcrEventGate((requestId) => invoke('screenshot_is_current', { requestId }));
globalThis.window = {
    crypto: webcrypto,
    __TAURI_IPC__(payload) {
        // Tauri's Rust InvokePayload deserializes these into numeric CallbackFn IDs.
        assert.equal(typeof payload.callback, 'number', 'business arguments overwrote Tauri callback');
        assert.equal(typeof payload.error, 'number', 'business arguments overwrote Tauri error callback');
        if (payload.cmd === 'screenshot_is_current') {
            window[`_${payload.callback}`](active && payload.requestId === 7);
            return;
        }
        assert.equal(payload.cmd, 'screenshot_publish');
        assert.equal(typeof payload.isError, 'boolean');
        if (failPublication || !active) {
            window[`_${payload.error}`]('Screenshot superseded');
            return;
        }
        const event = payload.isError ? 'recognize_error' : 'new_text';
        events.push(event);
        void gate.accept({ requestId: payload.requestId, text: payload.text }, (text) => displayed.push({ event, text }));
        window[`_${payload.callback}`](null);
    },
};

try {
    for (const failure of [false, true]) {
        await runOcrRequest({
            current: () => active,
            hide: async () => {},
            crop: async () => ({ path: failure ? 'failure.png' : 'success.png' }),
            show: async () => { displayed.push({ event: 'new_text', text: '' }); },
            recognize: async () => { if (failure) throw new Error('OCR failed'); return 'recognized text'; },
            publish,
            restore: async () => assert.fail('unexpected overlay restore'),
            reset: () => {},
            cleanup: async (path) => cleaned.push(path),
            noText: 'No text',
        });
    }
    assert.deepEqual(events, ['new_text', 'recognize_error']);
    assert.deepEqual(displayed, [
        { event: 'new_text', text: '' }, { event: 'new_text', text: 'recognized text' },
        { event: 'new_text', text: '' }, { event: 'recognize_error', text: 'OCR failed' },
    ]);
    assert.deepEqual(cleaned, ['success.png', 'failure.png']);
    failPublication = true;
    await assert.rejects(publish('stale text', false), (error) => error === 'Screenshot superseded');
    assert.deepEqual(events, ['new_text', 'recognize_error']);
    assert.equal(Object.getOwnPropertyNames(window).filter((key) => /^_\d+$/.test(key)).length, 0, 'invoke must release both callbacks');
} finally {
    if (originalWindow === undefined) delete globalThis.window;
    else globalThis.window = originalWindow;
}

// Catch the same reserved transport keys at other direct invoke call sites.
function checkDirectory(directory) {
    for (const item of readdirSync(directory, { withFileTypes: true })) {
        const path = new URL(item.name + (item.isDirectory() ? '/' : ''), directory);
        if (item.isDirectory()) { checkDirectory(path); continue; }
        if (!/\.(js|jsx|ts|tsx)$/.test(item.name) || item.name.endsWith('.test.js')) continue;
        const file = ts.createSourceFile(path.pathname, readFileSync(path, 'utf8'), ts.ScriptTarget.Latest, true);
        const scan = (node) => {
            if (ts.isCallExpression(node) && node.expression.getText(file) === 'invoke' && node.arguments[1] && ts.isObjectLiteralExpression(node.arguments[1])) {
                for (const property of node.arguments[1].properties) {
                    const name = property.name?.getText(file).replace(/['"]/g, '');
                    assert.ok(!['cmd', 'callback', 'error'].includes(name), `${path.pathname}: invoke argument '${name}' is reserved by Tauri`);
                }
            }
            ts.forEachChild(node, scan);
        };
        scan(file);
    }
}
checkDirectory(new URL('../', import.meta.url));
console.log('OCR production callback / real Tauri invoke / result consumption tests passed');
