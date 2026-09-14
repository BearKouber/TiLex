import { entryDisplay } from './saved_entry.js';
import { SENTENCE_CATEGORIES } from './translation_result.js';

// Shared display content, laid out as compact notes. Only this formatter adds
// Markdown syntax; saved/model text must remain literal in every field.

const isHan = (c) => /\p{Script=Han}/u.test(c);
const firstChar = (text) => [...(text ?? '').trim()][0] ?? '';

// 汉字按拼音首字母并入 A–Z：每个字母取拼音排序里的第一个字（吖 是整张表的第一个字），
// 取最后一个不大于它的边界字。没有 I/U/V 开头的拼音。
// ponytail: 边界字法，多音字按 ICU 默认读音（重→Z），不在拼音表里的生僻字会落到 Z；要准确读音再换拼音库。
const pinyin = new Intl.Collator('zh-Hans-CN');
const PINYIN_BOUNDARY = [...'吖八嚓哒妸发旮哈丌咔垃妈拏噢妑七呥仨他屲夕丫帀'];
const PINYIN_LETTERS = 'ABCDEFGHJKLMNOPQRSTWXYZ';

// A–Z 分组：英文按首字母，汉字按拼音首字母；数字、符号和日文（带假名）进 # 桶。
// 纯汉字的日文词（日本語）和中文分不开，按拼音归组。
const letterOf = (text) => {
    const c = firstChar(text);
    if (/^[A-Z]$/.test(c.toUpperCase())) return c.toUpperCase();
    if (!isHan(c) || /[\p{Script=Hiragana}\p{Script=Katakana}]/u.test(text)) return '#';
    const index = PINYIN_BOUNDARY.findLastIndex((boundary) => pinyin.compare(c, boundary) >= 0);
    return index < 0 ? '#' : PINYIN_LETTERS[index];
};

const pad2 = (n) => String(n).padStart(2, '0');
const stamp = (d) =>
    `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;

const hardBreak = '  \n';
const nonblank = (value) => typeof value === 'string' && value.trim().length > 0;

// Entities are decoded after Markdown parsing, so even line-start punctuation,
// backticks, links and HTML-looking source cannot become layout. Preserve visible
// indentation/repeated spaces with NBSP, and anchor empty lines with a zero-width
// character so hard breaks survive. Tabs retain their visible four-space stops.
export function markdownText(value) {
    if (typeof value !== 'string') return '';
    return value
        .replace(/\r\n?/g, '\n')
        .split('\n')
        .map((line) => {
            if (!line) return '&#8203;';
            let column = 0;
            const expanded = Array.from(line, (character) => {
                const width = character === '\t' ? 4 - (column % 4) : 1;
                column += width;
                return character === '\t' ? ' '.repeat(width) : character;
            }).join('');
            return expanded.replace(/[!-/:-@[-`{-~]| +/g, (part, offset) => {
                if (part[0] !== ' ') return `&#${part.codePointAt(0)};`;
                return part.length > 1 || offset === 0 || offset + part.length === expanded.length
                    ? '&#160;'.repeat(part.length)
                    : part;
            });
        })
        .join(hardBreak);
}

// Whole-field emphasis. Callers place it after a line start or a space and end the
// line with it, so entity punctuation at either edge still counts as flanking.
const italic = (value) => `*${markdownText(value.trim())}*`;
// Code spans never decode entities, so they take raw text: no backticks, one line.
const codeText = (value) => value.replaceAll('`', '').replace(/\s*[\r\n]\s*/g, ' ').trim();
// Keep every line of a multi-line field inside the blockquote.
const quote = (text) => text.split('\n').map((line) => `> ${line}`).join('\n');
const oneLine = (text) => (text ?? '').trim().replace(/\s*[\r\n]\s*/g, ' ');

function exampleBlocks(examples) {
    if (!examples.length) return [];
    return [
        '**例句**',
        examples
            .map(({ text, translation }, index) => {
                const content = markdownText(text) + hardBreak + italic(translation);
                const prefix = `${index + 1}. `;
                return prefix + content.replaceAll('\n', '\n' + ' '.repeat(prefix.length));
            })
            .join('\n'),
    ];
}

const noteBlocks = (notes) => (notes.length ? ['**补充说明**', ...notes.map(markdownText)] : []);

function wordBlocks(view, w, fromText) {
    const blocks = [];
    const symbols = view.pronunciations.map(({ symbol }) => codeText(symbol)).filter(Boolean);
    if (symbols.length) blocks.push(symbols.map((symbol) => `\`${symbol}\``).join(' · '));
    if (view.explanations.length) {
        const rows = [
            '| 词性 | 详细释义 |',
            '| :---: | :--- |',
        ];
        view.explanations.forEach(({ trait, explains }) => {
            const cleanTrait = nonblank(trait) ? trait.replace(/[`|\r\n]/g, ' ').trim() : '';
            const pos = cleanTrait ? `\`${cleanTrait}\`` : '';
            const meaning = explains.map(markdownText).join('；');
            rows.push(`| ${pos} | ${meaning} |`);
        });
        blocks.push(rows.join('\n'));
    } else if (nonblank(view.translation) || nonblank(w.translation)) {
        blocks.push(markdownText(view.translation || w.translation));
    }
    if (view.associations.length) {
        blocks.push(quote(`**常用搭配**：${view.associations.map(markdownText).join(' · ')}`));
    }
    blocks.push(...exampleBlocks(view.examples), ...noteBlocks(view.notes));
    if (fromText) blocks.push('出自：' + markdownText(fromText));
    return blocks;
}

// Every section appears only with content, so Google plain text and old records
// fall back to heading + source + translation without a separate branch.
function sentenceBlocks(view, s, children) {
    const blocks = [];
    if (view.difficulty) {
        const reason = nonblank(view.difficulty_reason) ? ' ' + markdownText(view.difficulty_reason) : '';
        blocks.push(`难度：${'★'.repeat(view.difficulty)}${reason}`);
    }
    if (nonblank(s.text)) blocks.push(italic(s.text));
    const translation = nonblank(view.translation) ? view.translation : s.translation;
    if (nonblank(translation)) blocks.push(quote(italic(translation)));

    const main = view.syntax_breakdown?.main_clause.trim() ?? '';
    if (main) {
        // Old long analyses span lines; keep them as text instead of one long code span.
        const trunk = /[\r\n]/.test(main) || !codeText(main) ? markdownText(main) : `\`${codeText(main)}\``;
        blocks.push(`**核心句型**：${trunk}`);
    }
    if (nonblank(view.syntax_breakdown?.clauses_and_modifiers)) {
        blocks.push(`**修饰成分**：${markdownText(view.syntax_breakdown.clauses_and_modifiers)}`);
    }
    if (nonblank(view.nuance_note)) blocks.push(`**语境说明**：${markdownText(view.nuance_note)}`);
    if (view.key_vocabulary.length) {
        const terms = view.key_vocabulary.map(
            ({ word, meaning_in_context }) => `\`${codeText(word)} (${codeText(meaning_in_context)})\``
        );
        blocks.push(`**重点术语**：${terms.join(' · ')}`);
    }
    blocks.push(...exampleBlocks(view.examples), ...noteBlocks(view.notes));
    if (children.length) blocks.push(`生词：${children.map((w) => markdownText(w.text)).join(' · ')}`);
    return blocks;
}

const SENTENCE_GROUPS = [...Object.keys(SENTENCE_CATEGORIES), null];

// Accept parsed details or SQLite JSON strings. The shared projection owns all
// validation, flattened-text deduplication and legacy association recovery.
// now is injectable; this function neither writes nor mutates its inputs.
export function buildMarkdown(entries, now = new Date()) {
    const visible = entries
        .filter((entry) => !entry.deleted)
        .map((entry) => ({
            ...entry,
            detail: typeof entry.detail === 'string' ? parseDetail(entry.detail) : entry.detail,
        }));
    // 单词：A–Z 后接 #；组内英文在前、中文在后，各按字母/拼音排。
    const words = visible
        .filter((e) => e.type === 'word')
        .map((e) => ({ ...e, letter: letterOf(e.text), han: isHan(firstChar(e.text)) }))
        .sort((a, b) => {
            if (a.letter !== b.letter) {
                if (a.letter === '#') return 1;
                if (b.letter === '#') return -1;
                return a.letter < b.letter ? -1 : 1;
            }
            if (a.han !== b.han) return a.han ? 1 : -1;
            return a.han ? pinyin.compare(a.text, b.text) : a.text.localeCompare(b.text);
        });
    // 长难句：按分类表顺序分组，未分类最后；组内难度升序（无难度最后），同难度新收藏在前。
    const sentences = visible
        .filter((e) => e.type !== 'word')
        .map((e) => ({ ...e, view: entryDisplay(e) }))
        .sort(
            (a, b) =>
                SENTENCE_GROUPS.indexOf(a.view.category) - SENTENCE_GROUPS.indexOf(b.view.category) ||
                (a.view.difficulty ?? 4) - (b.view.difficulty ?? 4) ||
                (b.created_at ?? 0) - (a.created_at ?? 0) ||
                b.id - a.id
        );
    const sentenceById = new Map(sentences.map((s) => [s.id, s]));

    const out = ['# 我的生词本', ''];
    out.push(`> 导出于 ${stamp(now)} · 单词 ${words.length} · 长难句 ${sentences.length}`, '');

    if (words.length > 0) {
        out.push('## 单词', '');
        let current = '';
        for (const w of words) {
            if (w.letter !== current) {
                current = w.letter;
                out.push(`### ${current === '#' ? '&#35;' : current}`, '');
            }
            out.push(`#### 📖 **${markdownText(oneLine(w.text))}**`, '');
            const from = sentenceById.get(w.source_id);
            for (const block of wordBlocks(entryDisplay(w), w, from && nonblank(from.text) ? from.text : null)) {
                out.push(block, '');
            }
        }
    }

    if (sentences.length > 0) {
        out.push('## 长难句', '');
        let current;
        for (const [index, s] of sentences.entries()) {
            if (s.view.category !== current) {
                current = s.view.category;
                const category = SENTENCE_CATEGORIES[current];
                out.push(`### ${category ? `${category.lang} · ${category.no} ${category.name}` : '未分类'}`, '');
            }
            const clean = s.text.trim().replace(/\s+/g, ' ');
            let summary = clean;
            if (clean.length > 35) {
                const cut = clean.slice(0, 35);
                const lastSpace = cut.lastIndexOf(' ');
                summary = (lastSpace > 20 ? cut.slice(0, lastSpace) : cut) + '...';
            }
            out.push(`#### ${index + 1}. ${italic(summary)}`, '');
            const children = words.filter((w) => w.source_id === s.id);
            for (const block of sentenceBlocks(s.view, s, children)) out.push(block, '');
            out.push('---', '');
        }
    }

    return out.join('\n');
}

// 模型爱在 JSON 外面套 ```json 围栏，也爱在前面加一句「好的，这是分析结果」。
// 取第一个 { 到最后一个 } 就够，解析不出来就当这次没跑过（PRD 说不阻塞入库）。
export const extractJson = (raw) => {
    const start = raw.indexOf('{');
    const end = raw.lastIndexOf('}');
    if (start === -1 || end <= start) return null;
    try {
        return JSON.parse(raw.slice(start, end + 1));
    } catch {
        return null;
    }
};

// 词句分流：trim 后按空白分词，词数 ≤ 2 且末尾不是句末标点 → word。
// 中文没有空格：带中文逗号/顿号/分号/冒号，或汉字超过 10 个，就算句子。
export const isWord = (text) => {
    const t = text.trim();
    if (/[，、；：]/.test(t) || (t.match(/\p{Script=Han}/gu) ?? []).length > 10) return false;
    return t.split(/\s+/).length <= 2 && !/[.?!。？！]$/.test(t);
};

// detail 是 JSON 字符串；AI 抽风写坏了不该让整页打不开。
export const parseDetail = (raw) => {
    try {
        return raw ? JSON.parse(raw) : null;
    } catch {
        return null;
    }
};

// 单词的 detail 就是悬浮窗存下来的谷歌词典结果，形状和 PopResult 的 DictView 一样。
export const wordSummary = (detail) =>
    entryDisplay({ detail })
        .explanations.map((e) => e.explains.join(', '))
        .join('; ');

// 列表过滤：type 筛选 + 原文/译文/释义三处一起搜，关键词已经 trim + 小写。
export const matchEntry = (entry, filter, keyword) => {
    if (filter !== 'all' && entry.type !== filter) return false;
    if (!keyword) return true;
    return `${entry.text} ${entry.translation ?? ''} ${wordSummary(entry.detail)}`.toLowerCase().includes(keyword);
};
