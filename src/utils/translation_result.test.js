import assert from 'node:assert/strict';
import { normalizeResult, SENTENCE_CATEGORIES } from './translation_result.js';

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

// Sentence tags: valid ones survive normalization; an invalid one drops alone.
const base = { kind: 'sentence', translation: 'Complete.' };
const tagged = normalizeResult({ ...base, category: ' zh03 ', difficulty: '2', difficulty_reason: ' 主谓分离 ' });
assert.equal(tagged.category, 'ZH03');
assert.equal(tagged.difficulty, 2);
assert.equal(tagged.difficulty_reason, '主谓分离');
assert.deepEqual(normalizeResult(tagged), tagged);
assert.equal(Object.keys(SENTENCE_CATEGORIES).length, 11);
assert.deepEqual(Object.keys(SENTENCE_CATEGORIES).slice(5, 7), ['EN06', 'ZH01']);
for (const category of ['EN07', 'ZH06', '', 3, null, {}, 'toString', '定语从句类']) {
    const result = normalizeResult({ ...base, category, difficulty: 1 });
    assert.equal(result.translation, 'Complete.', String(category));
    assert.equal('category' in result, false, String(category));
    assert.equal(result.difficulty, 1);
}
for (const difficulty of [0, 4, 1.5, -1, '', 'hard', true, null, [2], {}]) {
    const result = normalizeResult({ ...base, category: 'EN01', difficulty, difficulty_reason: 'why' });
    assert.equal(result.category, 'EN01', JSON.stringify(difficulty));
    assert.equal('difficulty' in result, false, JSON.stringify(difficulty));
    assert.equal('difficulty_reason' in result, false, 'reason needs a valid difficulty');
}
assert.equal('difficulty_reason' in normalizeResult({ ...base, difficulty: 3, difficulty_reason: ' ' }), false);
assert.equal('difficulty_reason' in normalizeResult({ ...base, difficulty: 3, difficulty_reason: 7 }), false);
assert.equal('category' in normalizeResult({ ...word, category: 'EN01' }), false, 'words carry no sentence tags');
console.log('translation result contract tests passed');
