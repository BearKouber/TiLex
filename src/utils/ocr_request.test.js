import assert from 'node:assert/strict';
import { runOcrRequest, createOcrEventGate, ocrErrorMessage } from './ocr_request.js';

const deferred = () => {
    let resolve, reject;
    const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
    return { promise, resolve, reject };
};
const tick = () => new Promise((resolve) => setImmediate(resolve));
for (const error of [new Error(), '', '  ', undefined, null, {}, { message: '' }]) {
    assert.equal(ocrErrorMessage(error, 'Localized failure'), 'Localized failure');
    assert.ok(ocrErrorMessage(error, '').trim());
    for (const beforeShow of [false, true]) {
        const outcomes = [];
        const fail = async () => { throw error; };
        await runOcrRequest({
            current: () => true, hide: async () => {},
            crop: beforeShow ? fail : async () => ({ path: 'empty-error.png' }),
            show: async () => {}, recognize: fail,
            publish: async (text, isError) => outcomes.push([text, isError]),
            restore: async (text) => outcomes.push([text, 'restore']), reset: () => {},
            cleanup: async (path) => outcomes.push(path), noText: 'No text', failureText: 'Localized failure',
        });
        assert.deepEqual(outcomes, beforeShow ? [['Localized failure', 'restore']] : [['Localized failure', true], 'empty-error.png']);
    }
}
for (const text of ['', ' \n\t', null]) {
    const events = [];
    await runOcrRequest({
        current: () => true, hide: async () => {}, crop: async () => ({ path: 'no-text.png' }),
        show: async () => {}, recognize: async () => text,
        publish: async (message, isError) => events.push([message, isError]),
        restore: async () => assert.fail('result already shown'), reset: () => assert.fail('no success reset'),
        cleanup: async (path) => events.push(path), noText: 'Localized no text',
    });
    assert.deepEqual(events, [['Localized no text', true], 'no-text.png']);
}
for (const staleStep of ['hide', 'crop', 'show', 'recognize', 'publish']) {
    for (const failure of [false, true]) {
        let active = true;
        const waiting = deferred();
        const calls = [];
        const options = {
            current: () => active,
            hide: async () => calls.push('hide'),
            crop: async () => { calls.push('crop'); return { path: 'A.png' }; },
            show: async () => calls.push('show'),
            recognize: async () => { calls.push('recognize'); return 'A'; },
            publish: async () => calls.push('publish'),
            restore: async () => calls.push('restore'),
            reset: () => calls.push('reset'),
            cleanup: async (path) => calls.push(`cleanup:${path}`),
            noText: 'No text',
        };
        options[staleStep] = () => { calls.push(staleStep); return waiting.promise; };
        const run = runOcrRequest(options);
        await tick();
        active = false; // B started, Escape/right-click, or unmount.
        if (failure) waiting.reject(new Error('late A failure'));
        else waiting.resolve(staleStep === 'crop' ? { path: 'A.png' } : 'A');
        await run;
        const tail = calls.slice(calls.indexOf(staleStep) + 1);
        assert.ok(tail.every((call) => call === 'cleanup:A.png'), `${staleStep}: ${tail}`);
        if (['show', 'recognize', 'publish'].includes(staleStep) || staleStep === 'crop' && !failure) {
            assert.ok(tail.includes('cleanup:A.png'));
        }
    }
}
for (const error of [false, true]) {
    let current = 1;
    const a = deferred();
    const results = [];
    const cleaned = [];
    const options = (id, result) => ({
        current: () => current === id,
        hide: async () => {}, crop: async () => ({ path: `${id}.png` }), show: async () => {},
        recognize: () => result, publish: async (text) => results.push(text), restore: async () => assert.fail('stale restore'),
        reset: () => results.push(`reset:${id}`), cleanup: async (path) => cleaned.push(path), noText: 'No text',
    });
    const first = runOcrRequest(options(1, a.promise));
    await tick();
    current = 2;
    await runOcrRequest(options(2, Promise.resolve('B')));
    if (error) a.reject(new Error('old error')); else a.resolve('A');
    await first;
    assert.deepEqual(results, ['B', 'reset:2']);
    assert.deepEqual(cleaned.sort(), ['1.png', '2.png']);
}
const checks = new Map();
const gate = createOcrEventGate((id) => { const check = deferred(); checks.set(id, check); return check.promise; });
const received = [];
const a = gate.accept({ requestId: 1, text: 'A' }, (text) => received.push(text));
const b = gate.accept({ requestId: 2, text: 'B' }, (text) => received.push(text));
assert.equal(gate.invalidate(1), false);
assert.equal(gate.invalidate(2), false);
await gate.accept({ requestId: 1, text: 'late A error' }, (text) => received.push(text));
checks.get(2).resolve(true); await b;
checks.get(1).resolve(true); await a;
assert.deepEqual(received, ['B']);
const c = gate.accept({ requestId: 3, text: 'C' }, (text) => received.push(text));
await gate.accept('normal selection', (text) => received.push(text));
checks.get(3).resolve(true); await c;
assert.deepEqual(received, ['B', 'normal selection']);
const d = gate.accept({ requestId: 4, text: 'cancelled' }, (text) => received.push(text));
gate.invalidate(5);
checks.get(4).resolve(true); await d;
assert.deepEqual(received, ['B', 'normal selection']);
console.log('OCR lifecycle and event queue tests passed');
