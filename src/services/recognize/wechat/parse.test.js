import assert from 'node:assert/strict';
import { parseWechatResponse } from './parse.js';

const encode = (blocks) => JSON.stringify({ errcode: 0, ocr_response: blocks });
assert.equal(parseWechatResponse(encode([{ top: 20, text: 'second' }, { top: 10, text: 'first' }, { text: ' ' }])), 'first\nsecond');
assert.equal(parseWechatResponse(encode([])), '');
assert.equal(parseWechatResponse(encode([{ text: '\n' }])), '');
assert.throws(() => parseWechatResponse('{"errcode":5}'), /errcode=5/);
for (const bad of ['', 'null', '[]', '{}', '{"errcode":"0"}', '{"errcode":0}', encode([null]), encode([{ text: 4 }]), encode([{ text: 'valid' }, {}]), `${encode([{ text: 'ok' }])} trailing`]) {
    assert.throws(() => parseWechatResponse(bad), (error) => error instanceof Error && error.message.length > 0);
}
console.log('WeChat OCR shape and business result tests passed');
