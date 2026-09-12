import assert from 'node:assert/strict';
import { cacheKey, getCached, setCached, requestConfigSnapshot } from './translate_cache.js';
import { buildAiRequest, DEFAULT_CUSTOM_INSTRUCTIONS, effectiveAiConfig } from '../services/translate/ai/instructions.js';

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

const aiKey = (raw, service = 'ai@instance') => key(requestConfigSnapshot(raw, service));
const aiConfig = {
    requestPath: 'https://example.test/v1', apiFormat: 'openai_chat', apiKey: 'fixture-key', model: 'fixture-model',
    customInstructions: 'Give an example.', requestArguments: { temperature: 0.2, metadata: { a: 1, b: 2 } },
};
assert.deepEqual(requestConfigSnapshot(aiConfig, 'ai@instance'), effectiveAiConfig(aiConfig));
assert.deepEqual(requestConfigSnapshot(aiConfig, 'ai'), requestConfigSnapshot(aiConfig, 'ai@instance'));
assert.equal(aiKey({}), aiKey({ customInstructions: DEFAULT_CUSTOM_INSTRUCTIONS }));
assert.notEqual(aiKey({}), aiKey({ customInstructions: '' }));
assert.equal(requestConfigSnapshot({ customInstructions: '' }, 'ai').customInstructions, '');
assert.notEqual(aiKey({ customInstructions: '' }), aiKey({ customInstructions: ' ' }));
assert.equal(aiKey({ promptList: [] }), aiKey({ customInstructions: '' }));

// Only values that reach a request affect identity. Settings labels, disabled
// controls, migration backups and parameter JSON key order cannot invalidate it.
const appearance = {
    ...aiConfig, instanceName: 'Renamed instance', icon: 'different-icon', iconLocked: true, enable: false,
    legacyPromptBackup: { promptList: [{ role: 'system', content: 'Archived prompt' }], requestArguments: '{broken' },
};
assert.equal(aiKey(aiConfig), aiKey(appearance));
assert.equal(aiKey(aiConfig), aiKey({ ...aiConfig,
    requestArguments: '{"metadata":{"b":2,"a":1},"temperature":0.2}' }));
assert.equal(aiKey(aiConfig), aiKey({ ...aiConfig,
    requestArguments: { ...aiConfig.requestArguments, model: 'ignored', messages: [], stream: true, response_format: {} } }));
const googleArgs = { ...aiConfig, apiFormat: 'google', requestArguments: { temperature: 0.2, unsupported: true } };
assert.equal(aiKey(googleArgs), aiKey({ ...googleArgs, requestArguments: { temperature: 0.2, unsupported: false } }));
assert.notEqual(aiKey(googleArgs), aiKey({ ...googleArgs, requestArguments: { temperature: 0.8 } }));
assert.notEqual(aiKey(aiConfig), aiKey({ ...aiConfig, requestArguments: { temperature: 0.2, stopTokens: ['a', 'b'] } }));
assert.notEqual(aiKey({ ...aiConfig, requestArguments: { fixture: ['a', 'b'] } }),
    aiKey({ ...aiConfig, requestArguments: { fixture: ['b', 'a'] } }));
for (const [field, value] of [
    ['customInstructions', 'Changed instructions'], ['model', 'new-model'], ['apiFormat', 'openai_responses'],
    ['requestPath', 'https://new.test/v1'], ['apiKey', 'rotated-fixture-key'],
    ['requestArguments', { temperature: 0.9 }], ['legacyReferenceInstructions', 'Changed migrated references'],
]) assert.notEqual(aiKey(aiConfig), aiKey({ ...aiConfig, [field]: value }));

// A new bundled prompt/default/structure version must have a different key,
// even if the stored config had no explicit custom instructions at all.
const defaultSnapshot = requestConfigSnapshot({}, 'ai');
assert.equal(defaultSnapshot.customInstructions, DEFAULT_CUSTOM_INSTRUCTIONS);
for (const field of ['instructionsVersion', 'promptVersion', 'resultSchemaVersion', 'internalPrompt']) {
    assert.ok(defaultSnapshot[field]);
    assert.notEqual(key(defaultSnapshot), key({ ...defaultSnapshot, [field]: `${defaultSnapshot[field]}-next` }));
}

const captured = requestConfigSnapshot(aiConfig, 'ai');
const capturedKey = key(captured);
const capturedBody = buildAiRequest('This is the original source.', 'en', 'zh', captured).body;
aiConfig.model = 'edited-after-start';
aiConfig.customInstructions = '';
aiConfig.requestArguments.metadata.a = 99;
assert.equal(captured.model, 'fixture-model');
assert.equal(captured.requestArguments.metadata.a, 1);
assert.deepEqual(buildAiRequest('This is the original source.', 'en', 'zh', captured).body, capturedBody);
setCached(capturedKey, 'late AI result with old settings');
assert.equal(getCached(aiKey(aiConfig)), undefined);
assert.equal(getCached(capturedKey), 'late AI result with old settings');

// Other providers retain their existing raw configuration snapshot behavior.
const otherConfig = { enable: false, appearance: 'retained', nested: { value: 1 } };
const otherSnapshot = requestConfigSnapshot(otherConfig, 'google@instance');
assert.deepEqual(otherSnapshot, otherConfig);
otherConfig.nested.value = 2;
assert.equal(otherSnapshot.nested.value, 1);

for (let i = 0; i < 201; i++) setCached(`lru-${i}`, i);
assert.equal(getCached('lru-0'), undefined);
assert.equal(getCached('lru-1'), 1);
setCached('lru-201', 201);
assert.equal(getCached('lru-2'), undefined);
assert.equal(getCached('lru-1'), 1);
console.log('translation cache identity/LRU tests passed');
