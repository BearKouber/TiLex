import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { createPopResultSizing } from './pop_result_sizing.js';
import { createOcrEventGate, ocrErrorMessage } from './ocr_request.js';
import { preprocess } from './text_preprocess.js';

// Execute the production effect and event handlers. Native APIs and layout are
// controlled boundaries; this tests wiring, not Windows/WebView2 painting.
const source = ts.createSourceFile(
    'PopResult.jsx',
    readFileSync(new URL('../window/PopResult/index.jsx', import.meta.url), 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.JSX
);
const expressions = {};
let effect;
function visit(node) {
    if (ts.isVariableDeclaration(node)) {
        const name = node.name.getText(source);
        if (['run', 'unlistenAnchor', 'unlistenText', 'unlistenSession', 'unlistenErr'].includes(name)) {
            assert.equal(expressions[name], undefined);
            expressions[name] = node.initializer.getText(source);
        }
    }
    if (
        ts.isCallExpression(node) &&
        node.expression.getText(source) === 'useEffect' &&
        node.arguments[0].getText(source).includes('createPopResultSizing(')
    ) {
        assert.equal(effect, undefined);
        effect = node.arguments[0].getText(source);
    }
    ts.forEachChild(node, visit);
}
visit(source);
assert.equal(Object.keys(expressions).length, 5);
assert.ok(effect, 'the actual component sizing effect must be exercised');

const events = new Map();
const sizes = [];
const paints = [];
// repaint 靠改 opacity 逼 WebView2 出帧：记下每次改成非空值的那一下。
const boxRef = { current: { offsetHeight: 80, style: { set opacity(v) { if (v) paints.push(v); } } } };
const sizingRef = { current: null };
let position = { x: 30, y: 600 };
let observer;
class ResizeObserver {
    constructor(callback) {
        this.callback = callback;
        observer = this;
    }
    observe(element) {
        assert.equal(element, boxRef.current);
    }
    disconnect() {
        this.disconnected = true;
    }
}
class LogicalSize {
    constructor(width, height) {
        this.width = width;
        this.height = height;
    }
}
class PhysicalPosition {
    constructor(x, y) {
        this.x = x;
        this.y = y;
    }
}
const bindings = {
    boxRef,
    sizingRef,
    ResizeObserver,
    LogicalSize,
    PhysicalPosition,
    entryRef: { current: null },
    blurRef: { current: { begin() {} } },
    width: 320,
    createPopResultSizing,
    appWindow: {
        setSize: async (size) => {
            assert.ok(size instanceof LogicalSize);
            assert.equal(size.width, 320);
            sizes.push(size.height);
        },
        outerPosition: async () => position,
        setPosition: async (value) => {
            assert.ok(value instanceof PhysicalPosition);
            position = value;
        },
        setFocus: async () => {},
    },
    currentMonitor: async () => ({ position: { x: 0, y: 0 }, size: { width: 1500, height: 1500 }, scaleFactor: 1.5 }),
    listen: (name, callback) => {
        events.set(name, callback);
        return Promise.resolve(() => events.delete(name));
    },
    createOcrEventGate,
    ocrErrorMessage,
    preprocess,
    traceOcr: () => {},
    t: (key) => key,
    setSource: () => {},
    setLang: () => {},
    setItems: () => {},
    setCollapsed: () => {},
    setSaved: () => {},
    setSavedKey: () => {},
    setStatus: () => {},
    flushSync: (fn) => fn(),
};
const setup = new Function(
    ...Object.keys(bindings),
    `
    const WIDTH = width;
    let runID = 0, origin = null, armed = false, awaitingText = false;
    const blur = { invalidate() {} };
    const gate = createOcrEventGate(async () => true);
    const run = ${expressions.run};
    ${Object.entries(expressions)
        .filter(([name]) => name !== 'run')
        .map(([name, expr]) => `const ${name} = ${expr};`)
        .join('\n')}
    return (${effect})();
`
);
const cleanup = setup(...Object.values(bindings));
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
async function emit(name, payload) {
    events.get(name)({ payload });
    await settle();
}

observer.callback(); // ResizeObserver 首次 observe 自带的那次回调
await settle();
assert.deepEqual(sizes, [80]);
assert.equal(paints.length, 1, 'every applied size forces a WebView frame');
await emit('pop_anchor', 700);
assert.equal(position.y, 600, 'the anchor alone never measures the previous result');
assert.deepEqual(sizes, [80]);
position = { x: 30, y: 600 };
await emit('new_text', '');
assert.equal(position.y, 580, 'same-height show rechecks its reused native window');
assert.deepEqual(sizes, [80, 80]);

boxRef.current.offsetHeight = 240;
observer.callback();
await settle();
assert.equal(position.y, 340);
await emit('pop_anchor', 500);
assert.equal(position.y, 340, 'the anchor alone never measures the previous result');
position = { x: 30, y: 600 };
await emit('recognize_error', 'controlled recognition error');
assert.equal(position.y, 140, 'the error display also refreshes same-height sizing');
assert.equal(paints.length, 4);
observer.callback();
await settle();
assert.equal(paints.length, 4, 'an unchanged height neither resizes nor repaints');

await emit('screenshot_session', { requestId: 10 });
boxRef.current.offsetHeight = 80;
observer.callback();
await settle();
assert.equal(sizes.at(-1), 240, 'session invalidation pauses obsolete content updates');
await emit('new_text', { requestId: 10, text: '' });
assert.equal(sizes.at(-1), 80, 'accepted new show resumes sizing after session invalidation');

cleanup();
assert.equal(observer.disconnected, true);
assert.equal(sizingRef.current, null);
const countAfterCleanup = sizes.length;
boxRef.current.offsetHeight = 240;
observer.callback();
await settle();
assert.equal(sizes.length, countAfterCleanup, 'an already queued callback cannot revive the disposed coordinator');

console.log('PopResult sizing production effect / show, anchor, error, session and cleanup wiring tests passed');
