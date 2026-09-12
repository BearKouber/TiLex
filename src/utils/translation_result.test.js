import assert from 'node:assert/strict';
import { normalizeResult } from './translation_result.js';

const word = {
    kind: 'word',
    schemaVersion: 999,
    pronunciations: [{ symbol: '/wɜːd/', voice: 'ignored' }],
    explanations: [{ trait: 'n.', explains: ['word', ''] }],
    associations: ['word choice', ' '],
    examples: [{ text: 'Choose a word.', translation: '选一个词。' }],
    notes: ['Usage', ' '],
};
const normalizedWord = normalizeResult(word, 'word');
assert.equal(normalizedWord.schemaVersion, 1);
assert.deepEqual(normalizedWord.explanations, [{ trait: 'n.', explains: ['word'] }]);
assert.deepEqual(normalizedWord.pronunciations, [{ symbol: '/wɜːd/' }]);
assert.deepEqual(normalizedWord.associations, ['word choice']);
assert.deepEqual(normalizedWord.notes, ['Usage']);
assert.deepEqual(normalizeResult(normalizedWord), normalizedWord);
assert.deepEqual(normalizeResult({ kind: 'sentence', translation: 'Complete.' }), {
    kind: 'sentence',
    schemaVersion: 1,
    translation: 'Complete.',
    examples: [],
    notes: [],
});
assert.deepEqual(normalizeResult({ kind: 'word', explanations: [{ explains: ['meaning'] }] }), {
    kind: 'word',
    schemaVersion: 1,
    explanations: [{ trait: undefined, explains: ['meaning'] }],
    pronunciations: [],
    associations: [],
    examples: [],
    notes: [],
});
const legacy = normalizeResult({ explanations: [{ explains: ['meaning'] }], sentence: [] });
assert.equal('kind' in legacy, false);
assert.equal('schemaVersion' in legacy, false);
assert.equal(normalizeResult(legacy, 'sentence'), null);
assert.equal(normalizeResult(word, 'sentence'), null);
assert.equal(normalizeResult({ kind: 'sentence', translation: 'Complete.' }, 'word'), null);

for (const invalid of [
    null,
    [],
    true,
    'text',
    {},
    { kind: 'other' },
    { kind: null },
    { kind: 'sentence' },
    { kind: 'sentence', translation: '' },
    { kind: 'sentence', translation: ' \n' },
    { kind: 'sentence', translation: 3 },
    { explanations: [] },
    { explanations: [{ explains: [' ', ''] }] },
    ...[
        null,
        {},
        ['text'],
        [null],
        [{ explains: false }],
        [{ explains: [3] }],
        [{ trait: [], explains: ['valid'] }],
    ].map((explanations) => ({ ...word, explanations })),
    ...[null, {}, 'bad', [null], [{ symbol: [] }]].map((pronunciations) => ({ ...word, pronunciations })),
    ...[null, {}, 'bad', [null], [3]].map((associations) => ({ ...word, associations })),
    ...[null, {}, 'bad', [null], [3]].map((notes) => ({ ...word, notes })),
    ...[
        null,
        {},
        'bad',
        [null],
        [{}],
        [{ text: 'x' }],
        [{ text: '', translation: 'x' }],
        [{ text: 'x', translation: '  ' }],
        [{ text: 'x', translation: {} }],
    ].map((examples) => ({ ...word, examples })),
    { kind: 'sentence', translation: 'Complete.', pronunciations: 'bad' },
])
    assert.equal(normalizeResult(invalid), null, JSON.stringify(invalid));
assert.deepEqual(normalizeResult({ ...legacy, examples: word.examples, notes: word.notes }).examples, word.examples);
console.log('translation result contract tests passed');
