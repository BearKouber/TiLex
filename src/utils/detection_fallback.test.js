import assert from 'node:assert/strict';
import { detectionWithFallback } from './detection_fallback.js';

assert.deepEqual(await detectionWithFallback(async () => 'ja', 'word', 'en'), { detected: 'ja', badge: 'ja' });
for (const from of ['auto', 'en']) {
    assert.deepEqual(await detectionWithFallback(async () => { throw new Error('offline'); }, 'word', from),
        { detected: from, badge: '' });
}
for (const result of [null, undefined, '', 'auto', {}]) {
    assert.deepEqual(await detectionWithFallback(async () => result, 'word'), { detected: 'auto', badge: '' });
}
let reject;
const detection = new Promise((_, fail) => { reject = fail; });
let dispatched = false;
const run = detectionWithFallback(() => detection, 'old').then(({ detected }) => {
    assert.equal(detected, 'auto');
    dispatched = true;
});
reject(new Error('request rejected'));
await run;
assert.equal(dispatched, true);
assert.equal((await detectionWithFallback(async () => 'en', 'next')).badge, 'en');
console.log('detection fallback tests passed');
