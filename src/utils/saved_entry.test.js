import assert from 'node:assert/strict';
import { test } from 'node:test';
import { createSavedEntry, entrySnapshot, entryDisplay, resultText } from './saved_entry.js';

const dictionary = {
    pronunciations: [{ symbol: '/a/' }, { symbol: '/b/' }],
    explanations: [{ trait: 'n.', explains: ['translation'] }],
    associations: ['machine translation', 'literal translation'],
};

const examples = [{ text: 'Literal example.', translation: '例句译文。' }];
const notes = ['Usage note.', 'Grammar note.'];
const word = { schemaVersion: 1, kind: 'word', ...dictionary, examples, notes };
const sentence = { schemaVersion: 1, kind: 'sentence', translation: '完整译文。', examples, notes };

function storage() {
    let nextId = 0;
    const rows = new Map();
    const calls = [];
    return {
        rows, calls,
        add: async (entry) => {
            calls.push({ action: 'add', entry });
            rows.set(++nextId, entry);
            return nextId;
        },
        update: async (id, entry) => {
            calls.push({ action: 'update', id, entry });
            // Match updateEntry's existing WHERE deleted=0 guard: a missing or
            // soft-deleted row is not recreated by a late translation result.
            if (rows.has(id)) rows.set(id, entry);
        },
        onStatus() {},
    };
}

test('dictionary saves preserve associations and display meanings only once', () => {
    const entry = entrySnapshot('translation', [{ result: dictionary }]);
    assert.deepEqual(entry.detail, dictionary);
    assert.equal(entryDisplay(entry).kind, 'word');
    assert.equal(entryDisplay(entry).translation, '');
    assert.deepEqual(entryDisplay(entry).associations, dictionary.associations);
    const legacy = { ...entry, detail: { pronunciations: dictionary.pronunciations, explanations: dictionary.explanations } };
    assert.deepEqual(entryDisplay(legacy), entryDisplay(entry));
    assert.equal(entryDisplay({ translation: 'plain sentence', detail: null }).translation, 'plain sentence');
});

test('new words and sentences preserve complete detail and render each section once', () => {
    for (const result of [word, sentence]) {
        const entry = entrySnapshot('original source', [{ key: 'ai@one', result }]);
        assert.deepEqual(entry.detail, result);
        const restored = { ...entry, detail: JSON.parse(JSON.stringify(entry.detail)) };
        const display = entryDisplay(restored);
        assert.equal(display.kind, result.kind);
        assert.equal(display.translation, result.kind === 'word' ? '' : sentence.translation);
        assert.deepEqual(display.examples, examples);
        assert.deepEqual(display.notes, notes);
        for (const text of [...examples.flatMap((example) => [example.text, example.translation]), ...notes]) {
            assert.equal(entry.translation.split(text).length - 1, 1);
            assert.equal(display.translation.includes(text), false);
        }
        assert.equal(entry.translation, resultText(result));
    }
    assert.equal(resultText(sentence), '完整译文。\nLiteral example.\n例句译文。\nUsage note.\nGrammar note.');
    assert.deepEqual(entrySnapshot('source', [{ result: { kind: 'sentence', translation: 'translation' } }]).detail,
        { schemaVersion: 1, kind: 'sentence', translation: 'translation', examples: [], notes: [] });
});

test('malformed objects never become dictionaries or crash display and selection', () => {
    const invalid = [
        null, undefined, [], [word], {}, { translation: 'not an explicit sentence' },
        { ...word, explanations: 'bad' }, { ...word, explanations: [null] },
        { ...word, examples: [{ text: 'example', translation: null }] },
        { ...sentence, notes: [false] }, { ...sentence, translation: {} },
        { ...word, kind: 'other' }, { ...dictionary, associations: {} },
    ];
    for (const value of invalid) {
        assert.equal(resultText(value), '');
        assert.deepEqual(entrySnapshot('source', [{ key: 'ai', result: value }, { result: 'fallback' }]),
            { text: 'source', translation: 'fallback', detail: null });
        const display = entryDisplay({ detail: value, translation: 'raw readable response' });
        assert.equal(display.kind, 'text');
        assert.equal(display.translation, 'raw readable response');
        assert.deepEqual(display.explanations, []);
    }
    const raw = '```json\n{"kind":"sentence","translation":null}\n```';
    assert.deepEqual(entrySnapshot('source', [{ key: 'ai', result: raw }]),
        { text: 'source', translation: raw, detail: null });
});

test('priority trusts the request service and selects one complete candidate', () => {
    const google = { key: 'google@one', result: dictionary };
    const ai = { key: 'ai@one', result: sentence };
    assert.deepEqual(entrySnapshot('source', [google, ai]).detail, sentence);
    assert.deepEqual(entrySnapshot('source', [ai, google]).detail, sentence);
    assert.deepEqual(entrySnapshot('source', [google, { ...ai, error: 'failed' }]).detail, dictionary);
    assert.deepEqual(entrySnapshot('source', [google, { ...ai, result: { ...sentence, examples: [], notes: [] } }]).detail, dictionary);
    // An AI sentence with only analysis or tags still beats a plain Google translation listed first.
    const bare = { ...sentence, examples: [], notes: [] };
    for (const extra of [{ category: 'EN01' }, { difficulty: 2 }, { nuance_note: 'Tone.' },
        { syntax_breakdown: { main_clause: 'Main' } }, { key_vocabulary: [{ word: 'term', meaning_in_context: '术语' }] }]) {
        const result = { ...bare, ...extra };
        assert.equal(entrySnapshot('source', [{ key: 'google@one', result: 'plain' }, { ...ai, result }]).detail.kind,
            'sentence', JSON.stringify(extra));
    }
    assert.equal(entrySnapshot('source', [{ key: 'google@one', result: 'plain' }, { ...ai, result: bare }]).detail, null);
    assert.deepEqual(entrySnapshot('source', [google, { key: 'plugin@one', result: { ...sentence, serviceName: 'ai' } }]).detail, dictionary);
    assert.deepEqual(entrySnapshot('source', [google, { serviceName: 'ai', result: sentence }]).detail, sentence);
    assert.deepEqual(entrySnapshot('source', [google, { meta: { serviceName: 'ai' }, result: sentence }]).detail, sentence);
    assert.deepEqual(entrySnapshot('source', [google, { key: 'ai', result: { ...dictionary, examples, notes } }]).detail, dictionary);
    const first = { kind: 'sentence', translation: 'First translation' };
    assert.equal(entrySnapshot('source', [{ result: first }, { result: 'later text' }]).translation, 'First translation');
    assert.equal(entrySnapshot('source', [{ result: 'first text' }, { result: first }]).translation, 'first text');
    assert.deepEqual(entrySnapshot('source', [{ result: 'first text' }, google]).detail, dictionary);
});

test('both Google/AI completion orders yield the same supplemented saved entry', async () => {
    for (const order of [['google', 'ai'], ['ai', 'google']]) {
        const db = storage();
        const session = createSavedEntry('source', db);
        session.setItems([{ key: 'google', result: '' }, { key: 'ai', result: '' }]);
        await session.patch(order[0], { result: order[0] === 'ai' ? sentence : dictionary });
        await session.save();
        await session.patch(order[1], { result: order[1] === 'ai' ? sentence : dictionary });
        assert.equal(db.rows.size, 1);
        assert.deepEqual(db.rows.get(1).detail, sentence);
    }
});

test('multiple AI instances choose initial service order despite reversed completion or later reorder', async () => {
    const first = { ...sentence, translation: 'First configured AI' };
    const second = { ...sentence, translation: 'Second configured AI' };
    for (const order of [['ai@first', 'ai@second'], ['ai@second', 'ai@first']]) {
        const db = storage();
        const session = createSavedEntry('source', db);
        session.setItems([{ key: 'google', result: dictionary }, { key: 'ai@first' }, { key: 'ai@second' }]);
        await session.save();
        for (const key of order) await session.patch(key, { result: key === 'ai@first' ? first : second });
        assert.deepEqual(db.rows.get(1).detail, first);
        await session.setItems([{ key: 'ai@second', result: second }, { key: 'ai@first', result: first }]);
        assert.deepEqual(db.rows.get(1).detail, first);
        assert.equal(db.rows.get(1).translation.includes(second.translation), false);
    }
});

test('legacy analysis remains safe and old word associations recover only from a matching prefix', () => {
    const analysis = {
        syntax_breakdown: { main_clause: 'Main clause', clauses_and_modifiers: 'Modifiers' },
        nuance_note: 'Legacy nuance',
        key_vocabulary: [{ word: 'word', meaning_in_context: 'Meaning' }],
    };
    const display = entryDisplay({ translation: 'Legacy sentence', detail: analysis });
    assert.equal(display.kind, 'text');
    assert.equal(display.translation, 'Legacy sentence');
    for (const [key, value] of Object.entries(analysis)) assert.deepEqual(display[key], value);
    const malformed = entryDisplay({ translation: 'readable', detail: {
        syntax_breakdown: { main_clause: {}, clauses_and_modifiers: null },
        nuance_note: [], key_vocabulary: [null, { word: {} }, { word: 'safe', meaning_in_context: {} }],
    } });
    assert.equal(malformed.syntax_breakdown, null);
    assert.equal(malformed.nuance_note, '');
    assert.deepEqual(malformed.key_vocabulary, []);
    const legacy = { pronunciations: dictionary.pronunciations, explanations: dictionary.explanations };
    assert.deepEqual(entryDisplay({ detail: legacy, translation: resultText(dictionary) }).associations, dictionary.associations);
    assert.deepEqual(entryDisplay({ detail: legacy, translation: 'unrelated translation\nnot a collocation' }).associations, []);
});

test('sentence tags reach the display; old, plain and word records have empty tags', () => {
    const tagged = entryDisplay({ translation: 'x', detail: {
        ...sentence, category: 'en02', difficulty: 2, difficulty_reason: '主谓被逗号隔开',
    } });
    assert.equal(tagged.category, 'EN02');
    assert.equal(tagged.difficulty, 2);
    assert.equal(tagged.difficulty_reason, '主谓被逗号隔开');
    const broken = entryDisplay({ translation: 'x', detail: { ...sentence, category: 'EN09', difficulty: 9 } });
    assert.equal(broken.translation, sentence.translation);
    for (const detail of [sentence, word, null, '{bad', { syntax_breakdown: { main_clause: 'Old' } }]) {
        const display = entryDisplay({ translation: 'plain', detail });
        assert.equal(display.category, null);
        assert.equal(display.difficulty, null);
        assert.equal(display.difficulty_reason, '');
    }
});

test('deleted saved rows receive updates only and cannot be reinserted by late AI', async () => {
    const db = storage();
    const session = createSavedEntry('source', db);
    session.setItems([{ key: 'google', result: dictionary }, { key: 'ai', result: '' }]);
    await session.save();
    db.rows.delete(1);
    await session.patch('ai', { result: sentence });
    await session.save();
    assert.equal(db.rows.size, 0);
    assert.equal(db.calls.filter((call) => call.action === 'add').length, 1);
    assert.ok(db.calls.slice(1).every((call) => call.action === 'update' && call.id === 1));
});

test('same original with different target requests retains independent saved results', async () => {
    const db = storage();
    const chinese = createSavedEntry('source', db);
    const japanese = createSavedEntry('source', db);
    chinese.setItems([{ key: 'ai', result: 'early Chinese' }]);
    japanese.setItems([{ key: 'ai', result: 'Japanese translation' }]);
    await chinese.save();
    await japanese.save();
    await chinese.patch('ai', { result: sentence });
    assert.equal(db.rows.size, 2);
    assert.deepEqual(db.rows.get(1).detail, sentence);
    assert.equal(db.rows.get(2).translation, 'Japanese translation');
});

test('repeated saves serialize writes while another selection can save independently', async () => {
    let release;
    let active = 0;
    let inserts = 0;
    let maximum = 0;
    const updates = [];
    const session = createSavedEntry('slow selection', {
        add: async () => {
            inserts++;
            maximum = Math.max(maximum, ++active);
            await new Promise((resolve) => { release = resolve; });
            active--;
            return 10;
        },
        update: async (id, entry) => {
            maximum = Math.max(maximum, ++active);
            await Promise.resolve();
            updates.push({ id, entry });
            active--;
        },
        onStatus() {},
    });
    session.setItems([{ key: 'google', result: dictionary }, { key: 'ai', result: '' }]);
    const first = session.save();
    await Promise.resolve();
    const duplicate = session.save();
    const late = session.patch('ai', { result: sentence });
    const otherDb = storage();
    const other = createSavedEntry('independent selection', otherDb);
    other.setItems([{ key: 'ai', result: 'independent translation' }]);
    await other.save();
    assert.equal(otherDb.rows.get(1).translation, 'independent translation');
    assert.equal(updates.length, 0);
    release();
    await Promise.all([first, duplicate, late]);
    assert.equal(inserts, 1);
    assert.equal(maximum, 1);
    assert.equal(updates.length, 2);
    assert.ok(updates.every(({ id, entry }) => id === 10 && entry.text === 'slow selection'));
    assert.deepEqual(updates.at(-1).entry.detail, sentence);
});

test('late results update one row, including results arriving during insertion', async () => {
    let release;
    const updates = [];
    let inserts = 0;
    const session = createSavedEntry('translation', {
        add: async (entry) => {
            inserts++;
            assert.equal(entry.translation, '');
            await new Promise((resolve) => { release = resolve; });
            return 42;
        },
        update: async (id, entry) => updates.push({ id, ...entry }),
        onStatus() {},
    });
    session.setItems([{ key: 'ai', result: '', loading: true }]);
    const first = session.save();
    await Promise.resolve();
    session.patch('ai', { result: dictionary, loading: false });
    release();
    await first;
    await session.save();
    assert.equal(inserts, 1);
    assert.equal(updates.at(-1).id, 42);
    assert.deepEqual(updates.at(-1).detail, dictionary);
});

test('separate selections keep late results on their own rows', async () => {
    let nextId = 0;
    const rows = new Map();
    const storage = {
        add: async (entry) => { rows.set(++nextId, entry); return nextId; },
        update: async (id, entry) => rows.set(id, entry),
        onStatus() {},
    };
    const a = createSavedEntry('A', storage);
    const b = createSavedEntry('B', storage);
    a.setItems([{ key: 'ai', result: '' }]);
    b.setItems([{ key: 'ai', result: 'B result' }]);
    await a.save();
    await b.save();
    a.patch('ai', { result: dictionary });
    await a.save();
    assert.equal(rows.size, 2);
    assert.equal(rows.get(1).translation, resultText(dictionary));
    assert.equal(rows.get(2).translation, 'B result');
});
