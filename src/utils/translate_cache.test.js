import assert from 'node:assert/strict';
import { cacheKey, getCached, setCached, requestConfigSnapshot } from './translate_cache.js';

const config = { model: 'a', promptList: [{ role: 'user', content: '$text' }], requestPath: '/v1', apiFormat: 'openai', requestArguments: '{}', apiKey: 'key' };
const key = (c = config, detected = 'en') => cacheKey('A sentence to translate.', 'auto', 'zh_cn', 'ai@1', c, detected);
setCached(key(), 'old');
assert.equal(getCached(key({ ...config })), 'old');
assert.equal(key({ ...config, nested: { a: 1, b: 2 } }), key({ ...config, nested: { b: 2, a: 1 } }));
for (const field of ['model', 'promptList', 'requestPath', 'apiFormat', 'requestArguments', 'apiKey']) {
    assert.equal(getCached(key({ ...config, [field]: field === 'promptList' ? [{ role: 'user', content: 'New: $text' }] : 'changed' })), undefined);
}
assert.equal(getCached(key(config, 'ja')), undefined);
assert.notEqual(key({ list: ['a', 'b'] }), key({ list: ['b', 'a'] }));
const snapshot = requestConfigSnapshot(config);
const before = key(snapshot);
config.model = 'new';
config.promptList[0].content = 'new prompt';
assert.equal(snapshot.model, 'a');
assert.equal(snapshot.promptList[0].content, '$text');
// A finishes after settings changed: writes only its captured identity.
setCached(before, 'late old result');
assert.equal(getCached(key()), undefined);
assert.equal(getCached(before), 'late old result');
for (let i = 0; i < 201; i++) setCached(`lru-${i}`, i);
assert.equal(getCached('lru-0'), undefined);
assert.equal(getCached('lru-1'), 1);
setCached('lru-201', 201);
assert.equal(getCached('lru-2'), undefined);
assert.equal(getCached('lru-1'), 1);
console.log('translation cache identity/LRU tests passed');
