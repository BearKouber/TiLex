// node src/utils/wordbook_format.test.js
import assert from 'node:assert/strict';
import React from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import Markdown from 'react-markdown';
import {
    buildMarkdown,
    extractJson,
    isWord,
    markdownText,
    matchEntry,
    parseDetail,
    wordSummary,
} from './wordbook_format.js';
import { createSavedEntry, entryDisplay, resultText } from './saved_entry.js';

const now = new Date(2026, 8, 12, 16, 30);
const dictionary = {
    schemaVersion: 1,
    kind: 'word',
    pronunciations: [{ symbol: '/ˈfɑːloʊ/' }, { symbol: '/ˈfɒləʊ/' }],
    explanations: [
        { trait: 'v.', explains: ['跟随', '听从'] },
        { trait: 'n.', explains: ['关注'] },
    ],
    associations: ['follow up', 'as follows', 'follow suit'],
    examples: [
        { text: 'Follow the instructions.', translation: '按照说明操作。' },
        { text: 'I follow her work.', translation: '我关注她的工作。' },
    ],
    notes: ['follow 后可直接接宾语。', 'follow up 表示继续跟进。'],
};
const sentenceDetail = {
    schemaVersion: 1,
    kind: 'sentence',
    translation: '请按照说明操作，并保留原文。',
    examples: [{ text: 'Keep the original text.', translation: '原文保持不变。' }],
    notes: ['这是祈使句。', '省略主语 you。'],
};
const entries = [
    { id: 1, type: 'word', text: 'follow', translation: resultText(dictionary), detail: dictionary, source_id: 2 },
    {
        id: 2,
        type: 'sentence',
        text: 'Please follow the instructions\nand keep the original text.',
        translation: resultText(sentenceDetail),
        detail: sentenceDetail,
    },
    { id: 3, type: 'word', text: '幂等', translation: '重复执行仍产生同样结果', detail: null },
    { id: 4, type: 'word', text: 'archive', translation: '存档', detail: null },
    { id: 5, type: 'word', text: '99 bottles', translation: '九十九瓶', detail: null },
    {
        id: 6,
        type: 'sentence',
        text: 'Network latency impacts collaboration.',
        translation: '网络延迟影响协作。',
        detail: {
            syntax_breakdown: {
                main_clause: 'latency impacts collaboration',
                clauses_and_modifiers: 'Network 修饰 latency',
            },
            nuance_note: 'impact 在这里作及物动词。',
            key_vocabulary: [{ word: 'latency', meaning_in_context: '网络延迟' }],
        },
    },
];

function nodes(tree, type) {
    return [...(tree.type === type ? [tree] : []), ...(tree.children ?? []).flatMap((child) => nodes(child, type))];
}
function visibleText(node) {
    if (node.type === 'text') return node.value.replaceAll('\u00a0', ' ').replaceAll('\u200b', '');
    if (node.type === 'break') return '\n';
    const separator = ['root', 'list', 'listItem'].includes(node.type) ? '\n\n' : '';
    return (node.children ?? []).map(visibleText).join(separator);
}
function render(markdown) {
    let tree;
    const capture = () => (parsed) => {
        tree = parsed;
    };
    const html = renderToStaticMarkup(React.createElement(Markdown, { children: markdown, remarkPlugins: [capture] }));
    return { tree, html, text: visibleText(tree) };
}
const count = (text, needle) => text.split(needle).length - 1;
const md = buildMarkdown(entries, now);
const rendered = render(md);

// Field hierarchy, alphabet buckets and formatter-owned headings only.
assert.ok(md.startsWith('# 我的生词本'));
assert.ok(rendered.text.includes('导出于 2026-09-12 16:30 · 单词 4 · 长难句 2'));
assert.deepEqual(nodes(rendered.tree, 'heading').map(visibleText), [
    '我的生词本',
    '单词',
    'A',
    'F',
    '#',
    '长难句',
    '1',
    '2',
]);
assert.ok(md.includes('**follow**\n\n'));
assert.ok(rendered.text.includes('/ˈfɑːloʊ/\n/ˈfɒləʊ/'));
assert.ok(rendered.text.includes('v. 跟随；听从\nn. 关注'));
assert.ok(rendered.text.includes('搭配：follow up · as follows · follow suit'));
assert.equal(count(rendered.text, '听从'), 1, 'flattened dictionary meanings must not repeat');
assert.ok(!md.includes('<details>') && !md.includes('<a id=') && !md.includes('查看原句'));
assert.ok(rendered.text.includes('重复执行仍产生同样结果'));

// Source relationships are plain complete text; supplement pairs and legacy fields survive.
assert.ok(rendered.text.includes('出自：' + entries[1].text));
assert.ok(rendered.text.includes('生词：follow'));
for (const detail of [dictionary, sentenceDetail]) {
    for (const example of detail.examples) {
        assert.ok(rendered.text.includes(example.text + '\n' + example.translation));
        assert.equal(count(rendered.text, example.translation), 1);
    }
    for (const note of detail.notes) assert.equal(count(rendered.text, note), 1);
}
assert.equal(count(rendered.text, sentenceDetail.translation), 1);
for (const expected of [
    '主干：latency impacts collaboration',
    '修饰：Network 修饰 latency',
    '语义说明：impact 在这里作及物动词。',
    '上下文词汇',
    'latency：网络延迟',
])
    assert.ok(rendered.text.includes(expected));

// No empty sections, invalid-field crashes, guessed new associations or duplicate old ones.
const legacy = { pronunciations: [{ symbol: '/legacy/' }], explanations: [{ trait: 'n.', explains: ['旧释义'] }] };
const oldEntry = {
    id: 7,
    type: 'word',
    text: 'legacy',
    detail: legacy,
    translation: '/legacy/\nn. 旧释义\nold association',
};
const legacyText = render(buildMarkdown([oldEntry], now)).text;
assert.ok(legacyText.includes('搭配：old association'));
assert.equal(count(legacyText, '旧释义'), 1);
assert.equal(count(legacyText, 'old association'), 1);
assert.deepEqual(entryDisplay(oldEntry).associations, ['old association']);
assert.ok(
    !render(buildMarkdown([{ ...oldEntry, detail: { ...legacy, associations: [] } }], now)).text.includes(
        'old association'
    )
);
for (const detail of [
    null,
    '{broken',
    '[]',
    [],
    {},
    { explanations: 'bad' },
    { explanations: [null] },
    { kind: 'sentence', translation: false },
    { ...dictionary, associations: {} },
]) {
    const fallback = render(
        buildMarkdown([{ id: 1, type: 'word', text: 'fallback', translation: '唯一译文', detail }], now)
    );
    assert.equal(count(fallback.text, '唯一译文'), 1);
    for (const label of ['例句', '补充说明', '搭配：', '主干', '上下文词汇']) assert.ok(!fallback.text.includes(label));
    assert.equal(wordSummary(detail), '');
}
assert.equal(
    buildMarkdown(
        entries.map((entry) => ({ ...entry, detail: JSON.stringify(entry.detail) })),
        now
    ),
    md
);

// Deleted sources/children are excluded even when the pure function receives unfiltered rows.
const deleted = render(
    buildMarkdown(
        [
            { ...entries[0], source_id: 9 },
            { id: 9, type: 'sentence', text: 'DELETED_SOURCE', deleted: 1 },
            { id: 10, type: 'word', text: 'DELETED_CHILD', source_id: 2, deleted: true },
            { ...entries[1] },
            { id: 11, type: 'word', text: 'orphan', source_id: 999 },
        ],
        now
    )
).text;
assert.ok(!deleted.includes('DELETED_') && !deleted.includes('出自：'));
assert.ok(deleted.includes('单词 2 · 长难句 1'));
const empty = render(buildMarkdown([], now));
assert.ok(empty.text.includes('单词 0 · 长难句 0'));
assert.equal(nodes(empty.tree, 'heading').length, 1);

// Render every dynamic position through the installed real Markdown parser and SSR.
// Blank lines and leading/repeated/trailing spaces must survive as visible text.
const special =
    '<vector> `inline` ```code``` *literal* _name_ [text](url) a|b \\path &lt; &amp; &#60;\r\n' +
    '# heading\r> quote\n- bullet\n+ plus\n1. numbered\n---\n```js\n    const user_id = a < b && c > d;  \n\n  return "[x](https://example.test)";\n```';
const normalizedSpecial = special.replace(/\r\n?/g, '\n');
const specialDetail = {
    ...dictionary,
    pronunciations: [{ symbol: special }],
    explanations: [{ trait: special, explains: [special] }],
    associations: [special],
    examples: [{ text: special, translation: special }],
    notes: [special],
    syntax_breakdown: { main_clause: special, clauses_and_modifiers: special },
    nuance_note: special,
    key_vocabulary: [{ word: special, meaning_in_context: special }],
};
const cases = [
    ['original word', { type: 'word', text: special }],
    ['original sentence', { type: 'sentence', text: special }],
    ['plain translation', { type: 'word', text: 'plain', translation: special }],
    ['structured word fields', { type: 'word', text: 'word', detail: specialDetail }],
    [
        'structured sentence fields',
        {
            type: 'sentence',
            text: 'sentence',
            detail: {
                ...sentenceDetail,
                translation: special,
                examples: [{ text: special, translation: special }],
                notes: [special],
            },
        },
    ],
    [
        'legacy sentence fields',
        {
            type: 'sentence',
            text: 'old sentence',
            detail: {
                syntax_breakdown: specialDetail.syntax_breakdown,
                nuance_note: special,
                key_vocabulary: specialDetail.key_vocabulary,
            },
        },
    ],
];
for (const [name, entry] of cases) {
    const output = render(buildMarkdown([{ id: 1, ...entry }], now));
    const expectedOccurrences =
        name === 'structured word fields'
            ? 12
            : name === 'structured sentence fields'
              ? 4
              : name === 'legacy sentence fields'
                ? 5
                : 1;
    assert.equal(count(output.text, normalizedSpecial), expectedOccurrences, name);
    for (const type of [
        'html',
        'link',
        'linkReference',
        'image',
        'imageReference',
        'code',
        'inlineCode',
        'thematicBreak',
    ]) {
        assert.equal(nodes(output.tree, type).length, 0, `${name}: source generated ${type}`);
    }
    assert.equal(nodes(output.tree, 'blockquote').length, 1, name); // Fixed export metadata only.
    assert.equal(nodes(output.tree, 'list').length, name.startsWith('structured') ? 1 : 0, name);
    assert.ok(
        !output.html.includes('<vector>') && !output.html.includes('<a ') && !output.html.includes('<code>'),
        name
    );
    assert.ok(output.html.includes('&lt;vector&gt;') && output.html.includes('&amp;lt;'), name);
}
const relationText = render(
    buildMarkdown(
        [
            { id: 1, type: 'word', text: special, source_id: 2 },
            { id: 2, type: 'sentence', text: special },
        ],
        now
    )
).text;
assert.equal(count(relationText, normalizedSpecial), 4, 'word/sentence originals and both relation fields');
for (const source of ['  leading  and   repeated  ', '\nleading blank\n\ntrailing blank\n', 'line\r\n    code\rline']) {
    assert.equal(render(markdownText(source)).text, source.replace(/\r\n?/g, '\n'));
}
assert.equal(render(markdownText('\tfirst\n  \tsecond')).text, '    first\n    second');

// Production save selection feeds export: late supplemented AI replaces one complete
// Google result on its original row, even when a newer selection is already saved.
const rows = new Map();
let id = 0;
const storage = {
    add: async (snapshot) => {
        rows.set(++id, { id, type: 'sentence', ...snapshot });
        return id;
    },
    update: async (key, snapshot) => {
        rows.set(key, { id: key, type: 'sentence', ...snapshot });
    },
    onStatus() {},
};
const original = createSavedEntry('Original request.', storage);
original.setItems([{ key: 'google', result: { explanations: [{ explains: ['GOOGLE_ONLY'] }] } }, { key: 'ai@first' }]);
await original.save();
const newer = createSavedEntry('Newer request.', storage);
newer.setItems([{ key: 'ai@first', result: 'NEWER_ONLY' }]);
await newer.save();
await original.patch('ai@first', { result: sentenceDetail });
const lateText = render(buildMarkdown([...rows.values()], now)).text;
assert.ok(lateText.includes(sentenceDetail.translation) && lateText.includes('NEWER_ONLY'));
assert.ok(!lateText.includes('GOOGLE_ONLY'));
assert.equal(count(lateText, sentenceDetail.examples[0].translation), 1);
assert.deepEqual(rows.get(1).detail, sentenceDetail);

// Existing helper contracts remain available to the AI service and Wordbook filter.
assert.deepEqual(extractJson('```json\n{"a":1}\n```'), { a: 1 });
assert.deepEqual(extractJson('好的，结果如下：{"a":[1,2]} 完毕'), { a: [1, 2] });
assert.equal(extractJson('抱歉我不能'), null);
assert.equal(extractJson('{坏掉的}'), null);
for (const word of ['hello', 'give up', '  book  ']) assert.ok(isWord(word));
for (const sentence of ['Hello.', 'I love you', '这是一个句子。']) assert.ok(!isWord(sentence));
assert.equal(parseDetail(null), null);
assert.equal(parseDetail('{oops'), null);
assert.deepEqual(parseDetail('{"a":1}'), { a: 1 });
const dict = { explanations: [{ trait: 'n.', explains: ['延迟', '潜伏'] }] };
assert.equal(wordSummary(dict), '延迟, 潜伏');
const word = { type: 'word', text: 'latency', translation: '', detail: dict };
const sentence = { type: 'sentence', text: 'Network latency matters.', translation: '网络延迟很重要。', detail: null };
assert.ok(matchEntry(word, 'all', ''));
assert.ok(!matchEntry(word, 'sentence', ''));
assert.ok(matchEntry(word, 'all', 'laten'));
assert.ok(matchEntry(word, 'all', '潜伏'));
assert.ok(matchEntry(sentence, 'all', '网络'));
assert.ok(!matchEntry(sentence, 'all', 'zzz'));
assert.ok(matchEntry({ ...word, detail: { explanations: {} } }, 'all', 'laten'));

console.log('wordbook_format: hierarchy, shared normalization, save integration and real Markdown SSR passed');
