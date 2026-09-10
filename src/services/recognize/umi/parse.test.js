import assert from 'node:assert';
import { parseUmiResponse } from './parse.js';

// 成功：多块按顺序换行拼，空块丢掉
assert.strictEqual(
    parseUmiResponse({ code: 100, data: [{ text: 'hello' }, { text: '  ' }, { text: 'world' }] }),
    'hello\nworld'
);
// 图里没字：空串，不是报错
assert.strictEqual(parseUmiResponse({ code: 101, data: 'No text found in image' }), '');
// 其他 code：把 data 原样抛出去
assert.throws(() => parseUmiResponse({ code: 102, data: '图片格式错误' }), /图片格式错误/);
// data 不是字符串时也得有句人话
assert.throws(() => parseUmiResponse({ code: 999 }), /code=999/);

console.log('umi parse: ok');
