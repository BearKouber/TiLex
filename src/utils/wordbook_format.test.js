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
    if (node.type === 'text' || node.type === 'inlineCode') return node.value.replaceAll('\u00a0', ' ').replaceAll('\u200b', '');
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
    '📖 archive',
    'F',
    '📖 follow',
    'M',
    '📖 幂等',
    '#',
    '📖 99 bottles',
    '长难句',
    '未分类',
    '1. Network latency impacts...',
    '2. Please follow the instructions and...',
]);
assert.ok(md.includes('#### 📖 **follow**\n\n'));
assert.ok(md.includes('`/ˈfɑːloʊ/` · `/ˈfɒləʊ/`'));
assert.ok(rendered.text.includes('跟随；听从'));
assert.ok(md.includes('> **常用搭配**：follow up · as follows · follow suit'));
assert.ok(!md.includes('[!NOTE]') && !md.includes('[!TIP]') && !md.includes('**译文**'));
// Whole-field italics still parse when the text ends with CJK punctuation.
const emphasized = nodes(rendered.tree, 'emphasis').map(visibleText);
for (const text of ['按照说明操作。', sentenceDetail.translation, '网络延迟影响协作。', entries[1].text])
    assert.ok(emphasized.includes(text), text);
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
    '核心句型：latency impacts collaboration',
    '修饰成分：Network 修饰 latency',
    '语境说明：impact 在这里作及物动词。',
    '重点术语：latency (网络延迟)',
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
assert.ok(legacyText.includes('old association'));
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
// Code spans (pronunciations, terms) and the word heading take one-line raw text instead.
const oneLineSpecial = normalizedSpecial.replace(/\s*\n\s*/g, ' ');
for (const [name, entry] of cases) {
    const output = render(buildMarkdown([{ id: 1, ...entry }], now));
    const expectedOccurrences = {
        'original word': 0,
        'structured word fields': 5,
        'structured sentence fields': 4,
        'legacy sentence fields': 3,
    }[name] ?? 1;
    assert.equal(count(output.text, normalizedSpecial), expectedOccurrences, name);
    if (name === 'original word') assert.ok(output.text.includes('📖 ' + oneLineSpecial), name);
    // Italic wrapping survives punctuation at both edges of the literal text.
    if (['original sentence', 'structured word fields', 'structured sentence fields'].includes(name)) {
        assert.ok(nodes(output.tree, 'emphasis').some((node) => visibleText(node) === normalizedSpecial), name);
    }

    // Unescaped dynamic markdown must never leak into raw html, links, or code blocks.
    for (const type of ['link', 'linkReference', 'image', 'imageReference', 'code']) {
        assert.equal(nodes(output.tree, type).length, 0, `${name}: source generated ${type}`);
    }
    // Formatter-owned structural elements
    const expectedThematicBreak = entry.type === 'sentence' ? 1 : 0;
    assert.equal(nodes(output.tree, 'thematicBreak').length, expectedThematicBreak, `${name}: thematicBreak`);

    // Export header, plus the association or translation quote.
    const expectedBlockquote = name === 'structured word fields' || name === 'structured sentence fields' ? 2 : 1;
    assert.equal(nodes(output.tree, 'blockquote').length, expectedBlockquote, `${name}: blockquote`);

    // Word: trait + pronunciation. Legacy sentence: one term; its multi-line trunk stays text.
    const expectedInlineCode = { 'structured word fields': 2, 'legacy sentence fields': 1 }[name] ?? 0;
    assert.equal(nodes(output.tree, 'inlineCode').length, expectedInlineCode, `${name}: inlineCode`);
    if (expectedInlineCode) {
        assert.ok(nodes(output.tree, 'inlineCode').some((node) => node.value.includes(oneLineSpecial.replaceAll('`', '').trim())));
    }

    assert.ok(!output.html.includes('<vector>') && !output.html.includes('<a '), name);
    if (expectedInlineCode === 0) {
        assert.ok(!output.html.includes('<code>'), name);
    }
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
assert.equal(count(relationText, normalizedSpecial), 3, 'sentence original and both relation fields');
assert.ok(relationText.includes('📖 ' + oneLineSpecial), 'word heading keeps the text on one line');
for (const source of ['  leading  and   repeated  ', '\nleading blank\n\ntrailing blank\n', 'line\r\n    code\rline']) {
    assert.equal(render(markdownText(source)).text, source.replace(/\r\n?/g, '\n'));
}
assert.equal(render(markdownText('\tfirst\n  \tsecond')).text, '    first\n    second');

// AC6 words: English first-letter and Chinese pinyin share A–Z; English before Chinese
// inside a letter; kana words and other scripts go to the final # bucket.
const wordHeadings = (texts) =>
    nodes(render(buildMarkdown(texts.map((text, i) => ({ id: i + 1, type: 'word', text })), now)).tree, 'heading')
        .map(visibleText)
        .slice(2)
        .map((heading) => heading.replace('📖 ', ''));
assert.deepEqual(wordHeadings(['实现', '日本語です', 'banana', '安装', 'apple', '爱', 'Zebra', '中国', '7-zip', 'すし', '阿']), [
    'A', 'apple', '阿', '爱', '安装', 'B', 'banana', 'S', '实现', 'Z', 'Zebra', '中国', '#', '7-zip', 'すし', '日本語です',
]);
// Each pinyin initial, including the first characters of the table (吖) and around each boundary.
const buckets = {
    A: '吖啊阿安奥', B: '八把不白', C: '擦嚓从错', D: '哒大的对', E: '妸饿额恩二', F: '发方飞夫',
    G: '旮个该国', H: '哈好会火', J: '丌机家就', K: '咔卡可快', L: '垃了来路旅', M: '妈吗们木幂',
    N: '拏那女你', O: '噢哦欧', P: '妑怕盘平', Q: '七起去全', R: '呥然人日', S: '仨三上是实',
    T: '他天同', W: '屲瓦我无为', X: '夕下想行', Y: '丫一有语', Z: '帀在中重做',
};
for (const [letter, chars] of Object.entries(buckets)) {
    for (const char of chars) assert.deepEqual(wordHeadings([char]), [letter, char], char);
}

// AC6/AC9 sentences: category table order, then 未分类; difficulty ascending (none last),
// newer first on ties; numbering continues across groups.
const tagged = (id, text, category, difficulty, created_at, extra = {}) => ({
    id,
    type: 'sentence',
    text,
    translation: `${text} 译`,
    created_at,
    detail: { kind: 'sentence', translation: `${text} 译`, category, difficulty, ...extra },
});
const sentenceMd = buildMarkdown(
    [
        tagged(1, 'EN02 hard.', 'EN02', 2, 100, { difficulty_reason: '主谓被逗号隔开' }),
        tagged(2, 'EN02 easy old.', 'EN02', 1, 50),
        tagged(3, 'EN02 easy new.', 'EN02', 1, 200),
        tagged(4, 'EN02 unrated.', 'EN02', undefined, 300),
        tagged(5, 'EN01 sentence.', 'en01', 3, 10),
        tagged(6, '在数字化转型的背景下，企业需要持续投入。', 'ZH02', 2, 10),
        { id: 7, type: 'sentence', text: 'Google only sentence.', translation: '谷歌译文。', detail: null, created_at: 5 },
        {
            id: 8,
            type: 'sentence',
            text: 'Legacy sentence.',
            translation: '旧译文。',
            created_at: 9,
            detail: { syntax_breakdown: { main_clause: '旧的长解析\n第二行', clauses_and_modifiers: '' } },
        },
        tagged(9, 'Bad tags.', 'EN07', 7, 1),
    ],
    now
);
const sentenceView = render(sentenceMd);
assert.deepEqual(nodes(sentenceView.tree, 'heading').map(visibleText).slice(2), [
    '英文 · 01 定语从句类',
    '1. EN01 sentence.',
    '英文 · 02 状语从句类',
    '2. EN02 easy new.',
    '3. EN02 easy old.',
    '4. EN02 hard.',
    '5. EN02 unrated.',
    '中文 · 02 长状语阻隔类',
    '6. 在数字化转型的背景下，企业需要持续投入。',
    '未分类',
    '7. Legacy sentence.',
    '8. Google only sentence.',
    '9. Bad tags.',
]);
assert.ok(sentenceView.text.includes('难度：★★ 主谓被逗号隔开'));
assert.ok(sentenceView.text.includes('难度：★★★'));
assert.equal(count(sentenceView.text, '难度：'), 5, 'unrated, legacy, Google and invalid tags show no difficulty');
// AC7: a Google plain sentence has only its heading, source and translation.
const section = (markdown, heading) => markdown.slice(markdown.indexOf(heading), markdown.indexOf('---', markdown.indexOf(heading)));
assert.equal(
    section(sentenceMd, '#### 8.'),
    '#### 8. *Google only sentence&#46;*\n\n*Google only sentence&#46;*\n\n> *谷歌译文。*\n\n'
);
// Old multi-line trunk stays readable text, and no empty section titles appear.
const legacySection = section(sentenceMd, '#### 7.');
assert.ok(legacySection.includes('**核心句型**：旧的长解析  \n第二行'));
for (const label of ['修饰成分', '语境说明', '重点术语', '例句', '补充说明', '难度', '生词']) {
    assert.ok(!legacySection.includes(label), label);
}

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
// AC8: Chinese has no spaces; punctuation or more than 10 Han characters means a sentence.
for (const word of ['实现', '画蛇添足', '中华人民共和国', '十个汉字刚好不算句子']) assert.ok(isWord(word), word);
for (const sentence of ['项目上线后，性能明显提升', '安装、配置', '注意：先备份', '在数字化转型背景下企业需要持续投入'])
    assert.ok(!isWord(sentence), sentence);
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
