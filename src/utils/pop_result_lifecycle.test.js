import assert from 'node:assert/strict';
import { createBlurGuard } from './pop_result_lifecycle.js';

let time = 1000;
let focused = true;
let nextTimer = 0;
let hidden = 0;
let focusReply;
const timers = new Map();
const guard = createBlurGuard({
    now: () => time,
    schedule: (callback, delay) => {
        timers.set(++nextTimer, { callback, delay });
        return nextTimer;
    },
    cancel: (id) => timers.delete(id),
    isFocused: () => focusReply ?? Promise.resolve(focused),
    hide: () => hidden++,
});
const fire = async () => {
    const [id, timer] = timers.entries().next().value;
    timers.delete(id);
    return timer.callback();
};
guard.begin(1, 1);
guard.blur();
assert.equal(timers.size, 1);
assert.equal(timers.get(nextTimer).delay, 300);
guard.begin(2, 2);
assert.equal(timers.size, 0, 'new display clears old timer');
guard.blur();
let resolve;
focusReply = new Promise((done) => {
    resolve = done;
});
const lateCheck = fire();
guard.begin(3, 3);
resolve(false);
await lateCheck;
assert.equal(hidden, 0, 'old focus response cannot hide a new display');
focusReply = undefined;
guard.blur();
await fire();
assert.equal(hidden, 0, 'focus handover remains visible');
focused = false;
guard.blur();
await fire();
assert.equal(hidden, 1, 'a real early blur still dismisses');
guard.begin(4, 4);
time += 301;
guard.blur();
assert.equal(hidden, 2, 'normal blur still dismisses immediately');
guard.begin(5, 5);
guard.blur();
guard.invalidate();
assert.equal(timers.size, 0, 'unmount/cancellation clears timer');
guard.begin(6, 6);
guard.blur();
focusReply = new Promise((done) => {
    resolve = done;
});
const unmountedCheck = fire();
guard.invalidate();
resolve(false);
await unmountedCheck;
assert.equal(hidden, 2, 'unmount also invalidates in-flight focus queries');
guard.begin(7, 7);
guard.blur();
guard.focus();
assert.equal(timers.size, 0, 'refocus clears an earlier blur timer');
guard.blur();
focusReply = new Promise((done) => {
    resolve = done;
});
const refocusedCheck = fire();
guard.focus();
resolve(false);
await refocusedCheck;
assert.equal(hidden, 2, 'a focus event supersedes an older false focus reply in the same run');
time += 301;
guard.blur();
assert.equal(hidden, 3, 'refocus does not extend the original grace period or disable a later blur');
console.log('PopResult blur generation / focus / cleanup tests passed');
