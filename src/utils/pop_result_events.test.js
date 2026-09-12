import assert from 'node:assert/strict';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { createRequire } from 'node:module';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { webcrypto } from 'node:crypto';
import ts from 'typescript';
import { createBlurGuard } from './pop_result_lifecycle.js';

// Execute the production registrations through the installed Tauri API and the
// locked Rust dependency's actual injected JS dispatcher. A hand-written event
// mock that calls only this window's handlers would hide the global-listen bug.
const lock = readFileSync(new URL('../../src-tauri/Cargo.lock', import.meta.url), 'utf8');
const version = lock.match(/\[\[package\]\]\r?\nname = "tauri"\r?\nversion = "([^"]+)"/)[1];
const registry = join(process.env.CARGO_HOME ?? join(homedir(), '.cargo'), 'registry', 'src');
const tauriSource = readdirSync(registry)
    .map((index) => join(registry, index, `tauri-${version}`, 'src'))
    .find((path) => existsSync(join(path, 'manager.rs')));
assert.ok(tauriSource, `Fetch the locked tauri ${version} source before this native event integration test`);

function rustTemplate(file, name) {
    const source = readFileSync(join(tauriSource, file), 'utf8');
    const match = source.match(new RegExp(`fn ${name}\\b[\\s\\S]*?format!\\(\\s*"([\\s\\S]*?)",`));
    assert.ok(match, `Tauri's ${name} template must be reviewed when its shape changes`);
    return (fields) =>
        match[1]
            .replace(/\{([a-z_]+)\}/g, (_, field) => {
                assert.ok(Object.hasOwn(fields, field), `missing template argument ${field}`);
                return fields[field];
            })
            .replaceAll('{{', '{')
            .replaceAll('}}', '}');
}
const emitScript = rustTemplate('manager.rs', 'event_initialization_script');
const listenScript = rustTemplate('event.rs', 'listen_js');
const unlistenScript = rustTemplate('event.rs', 'unlisten_js');
const source = ts.createSourceFile(
    'PopResult.jsx',
    readFileSync(new URL('../window/PopResult/index.jsx', import.meta.url), 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.JSX
);
const registration = {};
function visit(node) {
    if (ts.isVariableDeclaration(node) && ['unlistenBlur', 'unlistenFocus'].includes(node.name.getText(source))) {
        registration[node.name.getText(source)] = node.initializer.getText(source);
    }
    ts.forEachChild(node, visit);
}
visit(source);
assert.equal(Object.keys(registration).length, 2);

const originalWindow = globalThis.window;
const listenerLabels = [];
let eventId = 0;
globalThis.window = {
    crypto: webcrypto,
    __TAURI_METADATA__: { __currentWindow: { label: 'pop_result' }, __windows: [{ label: 'pop_result' }] },
    __TAURI_IPC__({ __tauriModule, message, callback }) {
        assert.equal(__tauriModule, 'Event');
        if (message.cmd === 'listen') {
            const id = ++eventId;
            listenerLabels.push(message.windowLabel);
            new Function(
                'window',
                listenScript({
                    listeners: '__reviewListeners',
                    event: JSON.stringify(message.event),
                    event_id: id,
                    window_label: JSON.stringify(message.windowLabel),
                    handler: `window._${message.handler}`,
                })
            )(window);
            window[`_${callback}`](id);
        } else {
            assert.equal(message.cmd, 'unlisten');
            new Function(
                'window',
                unlistenScript({
                    listeners_object_name: '__reviewListeners',
                    event_name: message.event,
                    event_id: message.eventId,
                })
            )(window);
            window[`_${callback}`](null);
        }
    },
};

try {
    const require = createRequire(import.meta.url);
    const { appWindow } = require('@tauri-apps/api/window');
    const { listen } = require('@tauri-apps/api/event');
    new Function('window', emitScript({ function: '__reviewEmit', listeners: '__reviewListeners' }))(window);
    const emit = (event, windowLabel) => window.__reviewEmit({ event: `tauri://${event}`, windowLabel, payload: null });
    let time = 1000;
    let hidden = 0;
    let timerId = 0;
    const timers = new Map();
    const blur = createBlurGuard({
        now: () => time,
        isFocused: async () => false,
        hide: () => hidden++,
        schedule: (callback) => {
            timers.set(++timerId, callback);
            return timerId;
        },
        cancel: (id) => timers.delete(id),
    });
    const unlisteners = await Promise.all(
        Object.values(registration).map((expression) =>
            new Function('appWindow', 'listen', 'blur', `return (${expression});`)(appWindow, listen, blur)
        )
    );
    assert.deepEqual(listenerLabels, ['pop_result', 'pop_result'], 'production native focus listeners must be scoped');

    // Positive control: the real global API receives other windows' events.
    let globalBlurs = 0;
    const stopGlobal = await listen('tauri://blur', () => globalBlurs++);
    for (const label of ['config', 'screenshot']) {
        emit('focus', label);
        emit('blur', label);
    }
    assert.equal(globalBlurs, 2);
    assert.equal(hidden, 0, 'foreign events cannot enqueue a hide before the first result is initialized');
    assert.equal(timers.size, 0);

    blur.begin(1, 3);
    emit('blur', 'pop_result');
    assert.equal(timers.size, 1);
    emit('focus', 'config');
    emit('focus', 'screenshot');
    assert.equal(timers.size, 1, 'another window gaining focus must not cancel the real dismissal check');
    await timers.get(timerId)();
    timers.delete(timerId);
    assert.equal(hidden, 1, 'real early blur still hides after the unchanged grace period');

    blur.begin(2, 4);
    emit('blur', 'pop_result');
    emit('focus', 'pop_result');
    assert.equal(timers.size, 0, 'this window regaining focus cancels its pending blur');
    time += 301;
    emit('blur', 'screenshot');
    assert.equal(hidden, 1, 'hiding the screenshot overlay cannot hide an already displayed result');
    emit('blur', 'pop_result');
    assert.equal(hidden, 2, 'a real late blur still dismisses immediately');

    await Promise.all([...unlisteners.map((unlisten) => unlisten()), stopGlobal()]);
    emit('blur', 'pop_result');
    assert.equal(hidden, 2, 'cleanup removes both production listeners from the real dispatcher');
} finally {
    if (originalWindow === undefined) delete globalThis.window;
    else globalThis.window = originalWindow;
}
console.log('PopResult production listeners / real Tauri API and event routing tests passed');
