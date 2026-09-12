import assert from 'node:assert/strict';
import { createConfigCoordinator } from './config_coordinator.js';

// Explicit scheduler lets us execute callbacks even after cancellation, as a
// queued webview callback can already have been dispatched when deletion wins.
function harness(initial = { list: ['a', 'b'], a: { enable: true }, b: {} }) {
    let state = { values: structuredClone(initial), generations: {}, deleted: [], revision: 0 };
    let fail = false;
    let nextCommitGate;
    const timers = [];
    const calls = [];
    const errors = [];
    const listeners = new Set();
    function window() {
        const client = createConfigCoordinator({
            schedule: (fn) => {
                timers.push(fn);
                return timers.length;
            },
            unschedule: () => {},
            onError: (e) => errors.push(e),
            read: async () => structuredClone(state),
            commit: async (operations) => {
                calls.push(operations);
                const gate = nextCommitGate;
                nextCommitGate = undefined;
                if (gate) await gate;
                if (fail) throw new Error('disk refused');
                for (const op of operations) {
                    if (op.kind !== 'setIfAbsent' && op.generation !== (state.generations[op.key] ?? 0))
                        throw new Error('stale');
                    if (op.kind === 'set' && state.deleted.includes(op.key) && !op.revive) throw new Error('removed');
                }
                for (const op of operations) {
                    if (op.kind === 'delete') {
                        delete state.values[op.key];
                        state.deleted.push(op.key);
                    }
                    if (op.kind === 'set') {
                        state.values[op.key] = op.value;
                        if (op.revive) state.deleted = state.deleted.filter((k) => k !== op.key);
                    }
                    if (op.kind === 'setIfAbsent' && !state.deleted.includes(op.key) && !(op.key in state.values))
                        state.values[op.key] = op.value;
                    if (op.kind === 'delete' || op.invalidate || op.revive)
                        state.generations[op.key] = (state.generations[op.key] ?? 0) + 1;
                }
                state.revision++;
                listeners.forEach((fn) => fn(structuredClone(state)));
                return structuredClone(state);
            },
        });
        client.accept(structuredClone(state));
        listeners.add(client.accept);
        return client;
    }
    return {
        window,
        timers,
        calls,
        errors,
        state: () => state,
        fail: (next) => {
            fail = next;
        },
        holdNextCommit: () => {
            let release;
            nextCommitGate = new Promise((resolve) => {
                release = resolve;
            });
            return release;
        },
        replace: (next) => {
            state = next;
        },
    };
}

{
    const h = harness();
    const c = h.window();
    c.set('a', { enable: false });
    c.set('list', ['b', 'a']);
    await c.removeService('list', 'a');
    for (const timer of h.timers) await timer();
    assert.deepEqual(h.state().values.list, ['b']);
    assert.equal(h.state().values.a, undefined);
    assert.equal(h.calls.length, 1, 'delete is one commit, cancelled callbacks do not invoke');
    const count = h.calls.length;
    assert.deepEqual(await c.initialize('a', {}, false), {});
    assert.deepEqual(await c.initialize('a', {}, true), {});
    assert.equal(h.calls.length, count, 'mounted defaults cannot resurrect deleted configuration');
    await c.saveService('list', 'a', { new: true }, 'a', true);
    assert.deepEqual(h.state().values.list, ['b', 'a']);
    assert.deepEqual(h.state().values.a, { new: true });
}
{
    const h = harness({ list: ['a', 'b', 'c', 'd'], a: {}, b: {}, c: {}, d: {} });
    const c = h.window();
    const release = h.holdNextCommit();
    const saved = c.set('list', ['d', 'c', 'b', 'a']);
    const dispatched = h.timers[0]();
    await Promise.resolve();
    assert.equal(h.calls.length, 1, 'reorder has entered IPC before deletion');
    assert.deepEqual(c.value('list'), ['d', 'c', 'b', 'a'], 'in-flight reorder remains the latest draft');
    const deleted = c.removeService('list', 'a');
    release();
    await Promise.all([saved, dispatched, deleted]);
    assert.deepEqual(h.state().values.list, ['d', 'c', 'b'], 'delete retains the dispatched reorder');
    assert.equal(h.state().values.a, undefined);
}
{
    const h = harness();
    const c = h.window();
    const release = h.holdNextCommit();
    const first = c.set('a', { value: 1 }, true);
    await Promise.resolve();
    const second = c.set('a', { value: 2 }, true);
    release();
    await first;
    assert.deepEqual(c.value('a'), { value: 2 }, 'earlier settlement cannot retire a newer in-flight draft');
    await second;
    assert.deepEqual(h.state().values.a, { value: 2 });
    h.fail(true);
    await assert.rejects(c.set('a', { value: 3 }, true), /disk refused/);
    assert.deepEqual(c.value('a'), { value: 2 }, 'failed in-flight draft rolls back');
}
{
    const h = harness();
    const a = h.window();
    const b = h.window();
    b.set('a', { old: true });
    b.set('list', ['a', 'b']);
    await a.removeService('list', 'a');
    for (const timer of h.timers) await timer();
    assert.equal(h.calls.length, 1, 'other-window timers are invalidated by committed generation');
    assert.equal(b.value('a'), undefined);
}
{
    const h = harness();
    const c = h.window();
    const observed = [];
    c.subscribe('list', (v) => observed.push(v));
    c.set('list', ['b', 'a']);
    h.fail(true);
    await assert.rejects(c.removeService('list', 'a'), /disk refused/);
    assert.deepEqual(c.value('list'), ['a', 'b']);
    assert.deepEqual(observed.at(-1), ['a', 'b'], 'save rejection restores mounted state too');
    assert.deepEqual(c.value('a'), { enable: true });
    h.fail(false);
    await c.saveService('list', 'a', { recovered: true });
    assert.deepEqual(c.value('a'), { recovered: true });
}
{
    const h = harness({});
    const c = h.window();
    await Promise.all([c.set('one', 'new', true), c.set('two', 2, true), c.initialize('one', 'old')]);
    assert.deepEqual(h.state().values, { one: 'new', two: 2 });
    assert.equal(await c.initialize('one', 'old'), 'new');
    assert.equal(await c.initialize('draft', 'only local', false), 'only local');
    assert.equal(h.state().values.draft, undefined);
}
{
    const h = harness();
    const c = h.window();
    const observed = [];
    c.subscribe('a', (v) => observed.push(v));
    h.replace({ values: { list: ['b'], b: {} }, generations: { a: 1, list: 1 }, deleted: ['a'], revision: 4 });
    await c.refresh();
    c.accept({ values: { a: 'late response' }, generations: {}, deleted: [], revision: 2 });
    assert.equal(c.value('a'), undefined);
    assert.equal(observed.at(-1), undefined);
    await assert.rejects(c.saveService('list', 'a', { stale: true }), /removed/);
}
console.log('config coordinator: deletion, cross-window timers, rollback, defaults and snapshots passed');
