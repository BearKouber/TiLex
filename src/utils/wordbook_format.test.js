// node src/utils/wordbook_format.test.js
import assert from 'node:assert';
import { buildMarkdown, extractJson, isWord, matchEntry, parseDetail, wordSummary } from './wordbook_format.js';

const entries = [
    {
        id: 1,
        type: 'sentence',
        text: 'Network latency impacts collaboration.',
        translation: '网络延迟影响协作。',
        source_id: null,
        detail: {
            syntax_breakdown: { main_clause: 'latency impacts collaboration', clauses_and_modifiers: '无从句' },
            nuance_note: '注意 impact 作及物动词',
        },
    },
    {
        id: 2,
        type: 'word',
        text: 'latency',
        translation: '',
        source_id: 1,
        detail: { pronunciations: [{ symbol: '/ˈleɪtənsi/' }], explanations: [{ trait: 'n.', explains: ['延迟'] }] },
    },
    { id: 3, type: 'word', text: '幂等', translation: '幂等的', source_id: null, detail: null },
];

const md = buildMarkdown(entries, new Date(2026, 8, 8, 16, 30));

assert.ok(md.startsWith('# 我的生词本'));
assert.ok(md.includes('导出于 2026-09-08 16:30 · 单词 2 · 长难句 1'));

// 锚点全部清干净 —— 用户的编辑器会把它们当纯文本显示出来
assert.ok(!md.includes('<a id='));
assert.ok(!md.includes('返回顶部'));
assert.ok(!md.includes('查看原句'));

// 一个词一行：列表项，不是标题
assert.ok(md.includes('- **latency** `/ˈleɪtənsi/` — *n.* 延迟'));
assert.ok(!md.includes('#### latency'));

// 词 → 句：直接把原句写在词底下，不再跳转
assert.ok(md.includes('  - 出自：Network latency impacts collaboration.'));
// 句 → 词：列出来就行，不再做成链接
assert.ok(md.includes('生词：latency'));
assert.ok(!md.includes('[latency](#word-2)'));

// 字母分组还在（大纲面板靠它导航），但不再带锚点
assert.ok(md.includes('### L'));
assert.ok(md.includes('### #'));
assert.ok(!md.includes('letter-'));

// 语法拆解仍然折叠（<details> 是块级元素，和行内 <a id> 不同）
assert.ok(md.includes('<details>') && md.includes('</details>'));

// 没有 detail 的词：translation 才是唯一释义来源，要印
assert.ok(md.includes('- **幂等** — 幂等的'));

// 有 detail 的词：translation 和 detail 同源，不能再印一遍（旧版的真 bug）
const dup = buildMarkdown(
    [
        {
            id: 9,
            type: 'word',
            text: 'banner',
            translation: 'ˈbanər\n名词 旗帜, 旗',
            source_id: null,
            detail: { pronunciations: [{ symbol: 'ˈbanər' }], explanations: [{ trait: '名词', explains: ['旗帜', '旗'] }] },
        },
    ],
    new Date(2026, 8, 8)
);
assert.strictEqual(dup.split('旗帜').length - 1, 1, '释义只能出现一次');

// 空库也要出一份能打开的文件
assert.ok(buildMarkdown([], new Date(2026, 8, 8)).includes('# 我的生词本'));

// AI 返回值的围栏和废话都要能剥掉
assert.deepStrictEqual(extractJson('```json\n{"a":1}\n```'), { a: 1 });
assert.deepStrictEqual(extractJson('好的，结果如下：{"a":[1,2]} 完毕'), { a: [1, 2] });
assert.strictEqual(extractJson('抱歉我不能'), null);
assert.strictEqual(extractJson('{坏掉的}'), null);

assert.ok(isWord('hello'));
assert.ok(isWord('give up'));
assert.ok(isWord('  book  '));
assert.ok(!isWord('Hello.'));
assert.ok(!isWord('I love you'));
assert.ok(!isWord('这是一个句子。'));
assert.strictEqual(parseDetail(null), null);
assert.strictEqual(parseDetail('{oops'), null);
assert.deepStrictEqual(parseDetail('{"a":1}'), { a: 1 });

const dict = { explanations: [{ trait: 'n.', explains: ['延迟', '潜伏'] }, {}] };
assert.strictEqual(wordSummary(dict), '延迟, 潜伏');
assert.strictEqual(wordSummary(null), '');

const word = { type: 'word', text: 'latency', translation: '', detail: dict };
const sentence = { type: 'sentence', text: 'Network latency matters.', translation: '网络延迟很重要。', detail: null };
assert.ok(matchEntry(word, 'all', ''));
assert.ok(!matchEntry(word, 'sentence', ''));
assert.ok(matchEntry(word, 'all', 'laten'));
assert.ok(matchEntry(word, 'all', '潜伏'));
assert.ok(matchEntry(sentence, 'all', '网络'));
assert.ok(!matchEntry(sentence, 'all', 'zzz'));

console.log('wordbook_format: ok');
