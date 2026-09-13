// node src/utils/wordbook_selection.test.js
import assert from 'node:assert/strict';
import { removeFromWordbook, softDeleteWordbookEntries, visibleWordbookEntries } from './wordbook_selection.js';

const entries = [5, 4, 3, 2, 1].map((id) => ({
    id,
    text: `word ${id}`,
    translation: id % 2 ? 'odd' : 'even',
    type: id % 2 ? 'word' : 'sentence',
}));
const initial = { entries, keyword: '', filter: 'all', selectedId: 3 };
for (const [selectedId, deleted, expected] of [
    [3, [3], 2],
    [5, [5], 4],
    [1, [1], 2],
    [3, [3, 2], 1],
    [3, [3, 1], 2],
    [3, [3, 2, 1], 4],
    [3, [5, 1], 3],
    [3, [5, 4, 3, 2, 1], null],
    [null, [5], 4],
]) {
    const result = removeFromWordbook({ ...initial, selectedId }, deleted);
    assert.equal(result.selectedId, expected);
    assert.deepEqual(
        result.entries.map((entry) => entry.id),
        entries.map((entry) => entry.id).filter((id) => !deleted.includes(id))
    );
}

let next = initial;
for (const expected of [2, 1, 4, 5, null]) {
    next = removeFromWordbook(next, [next.selectedId]);
    assert.equal(next.selectedId, expected, 'continuous deletion follows surviving visible order');
}

for (const filtering of [{ keyword: ' ODD ' }, { filter: 'word' }]) {
    const view = { ...initial, ...filtering };
    assert.deepEqual(
        visibleWordbookEntries(view).map((entry) => entry.id),
        [5, 3, 1]
    );
    assert.equal(removeFromWordbook(view, [3]).selectedId, 1);
    assert.equal(removeFromWordbook({ ...view, selectedId: 1 }, [1]).selectedId, 3);
    const empty = removeFromWordbook(view, [5, 3, 1]);
    assert.equal(empty.selectedId, null);
    assert.deepEqual(visibleWordbookEntries(empty), []);
    assert.deepEqual(
        empty.entries.map((entry) => entry.id),
        [4, 2],
        'hidden records survive'
    );
}
assert.equal(removeFromWordbook({ ...initial, keyword: 'word 3' }, [3]).selectedId, null);
assert.equal(removeFromWordbook({ ...initial, entries: [] }, [3]).selectedId, null);
assert.equal(initial.selectedId, 3);
assert.equal(entries.length, 5, 'selection computation must not mutate input');

const calls = [];
const db = { execute: async (...args) => calls.push(args) };
await softDeleteWordbookEntries(db, []);
assert.deepEqual(calls, [], 'an empty selection cannot issue an unbounded update');
await softDeleteWordbookEntries(db, [3, 1, 3]);
assert.deepEqual(calls, [['UPDATE entries SET deleted=1 WHERE id IN ($1, $2)', [3, 1]]]);
const failure = new Error('database rejected');
await assert.rejects(
    softDeleteWordbookEntries(
        {
            execute: async () => {
                throw failure;
            },
        },
        [3, 1]
    ),
    (error) => error === failure
);

console.log('wordbook_selection: visible-order continuation, filtering, batches, empty state and atomic SQL passed');
