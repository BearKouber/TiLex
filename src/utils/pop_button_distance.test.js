import assert from 'node:assert/strict';
import { normalizePopButtonDistance } from './pop_button_distance.js';

for (const value of [0, 4, 10, 19, 20]) {
    assert.equal(normalizePopButtonDistance(value), value, 'integer gaps in range are preserved');
}
for (const value of [
    undefined,
    null,
    false,
    true,
    '',
    '20',
    [],
    [20],
    {},
    4.5,
    -0.5,
    20.5,
    NaN,
    Infinity,
    -Infinity,
]) {
    assert.equal(normalizePopButtonDistance(value), 10, 'missing or malformed values use the default gap');
}
for (const value of [-1, -100, -Number.MAX_VALUE]) {
    assert.equal(normalizePopButtonDistance(value), 0, 'negative integers clamp to zero');
}
for (const value of [21, 37, 100, 1000, Number.MAX_VALUE]) {
    assert.equal(normalizePopButtonDistance(value), 20, 'large integers clamp to the maximum');
}

console.log('pop_button_distance: all tests passed');
