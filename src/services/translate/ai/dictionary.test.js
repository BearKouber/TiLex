import assert from 'node:assert/strict';
import { dictionaryResult } from './dictionary.js';
import { entrySnapshot, resultText } from '../../../utils/saved_entry.js';

const valid = { explanations: [{ trait: 'n.', explains: ['word'] }], pronunciations: [{ symbol: '/w/' }], associations: ['words'] };
assert.deepEqual(dictionaryResult(JSON.stringify(valid)), valid);
assert.deepEqual(dictionaryResult('```json\n' + JSON.stringify(valid) + '\n```'), valid);
for (const text of [JSON.stringify([valid]), '```json\n' + JSON.stringify([valid]) + '\n```']) {
    assert.equal(dictionaryResult(text), text);
}
assert.deepEqual(dictionaryResult(JSON.stringify({ explanations: [{ explains: [] }] })), {
    explanations: [{ trait: undefined, explains: [] }], pronunciations: [], associations: [],
});
const bad = [null, [], true, 'string', {}, { explanations: [] },
    { explanations: {} }, { explanations: [null] }, { explanations: ['word'] },
    { explanations: [{ explains: 'word' }] }, { explanations: [{ explains: [{}] }] },
    { explanations: [{ explains: ['word'], trait: {} }] },
    ...[null, {}, [null], [{}], [{ symbol: {} }]].map((pronunciations) => ({ ...valid, pronunciations })),
    ...[null, {}, 'word', [null], [{}]].map((associations) => ({ ...valid, associations })),
];
for (const value of bad) {
    const text = JSON.stringify(value);
    const result = dictionaryResult(text);
    assert.equal(result, text);
    assert.equal(resultText(result), text);
    assert.equal(entrySnapshot('word', [{ result }]).translation, text);
}
assert.equal(dictionaryResult('not JSON'), 'not JSON');
assert.match(resultText(dictionaryResult(JSON.stringify(valid))), /word/);
assert.deepEqual(entrySnapshot('word', [{ result: dictionaryResult(JSON.stringify(valid)) }]).detail, valid);
console.log('dictionary contract/copy/save tests passed');
