import { entryDisplay } from './saved_entry.js';

// Shared display content, laid out as compact notes. Only this formatter adds
// Markdown syntax; saved/model text must remain literal in every field.

// A–Z 分组；数字、中文、符号开头的统统进 # 桶。
const letterOf = (text) => {
    const c = (text ?? '').trim().charAt(0).toUpperCase();
    return c >= 'A' && c <= 'Z' ? c : '#';
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

function resultBlocks(view) {
    const blocks = [];
    if (view.pronunciations.length) {
        blocks.push(view.pronunciations.map(({ symbol }) => markdownText(symbol)).join(hardBreak));
    }
    if (view.explanations.length) {
        blocks.push(
            view.explanations
                .map(({ trait, explains }) => {
                    const label = nonblank(trait) ? `*${markdownText(trait)}* ` : '';
                    return label + explains.map(markdownText).join('；');
                })
                .join(hardBreak)
        );
    }
    if (nonblank(view.translation)) blocks.push(markdownText(view.translation));
    if (view.associations.length) blocks.push('搭配：' + view.associations.map(markdownText).join(' · '));
    if (view.examples.length) {
        blocks.push('**例句**');
        blocks.push(
            view.examples
                .map(({ text, translation }, index) => {
                    const content = markdownText(text) + hardBreak + markdownText(translation);
                    // Every continuation belongs to this formatter-owned numbered item.
                    const prefix = `${index + 1}. `;
                    return prefix + content.replaceAll('\n', '\n' + ' '.repeat(prefix.length));
                })
                .join('\n')
        );
    }
    if (view.notes.length) blocks.push('**补充说明**', ...view.notes.map(markdownText));
    const syntax = view.syntax_breakdown;
    if (nonblank(syntax?.main_clause)) blocks.push('**主干**：' + markdownText(syntax.main_clause));
    if (nonblank(syntax?.clauses_and_modifiers)) blocks.push('**修饰**：' + markdownText(syntax.clauses_and_modifiers));
    if (nonblank(view.nuance_note)) blocks.push('**语义说明**：' + markdownText(view.nuance_note));
    if (view.key_vocabulary.length) {
        blocks.push('**上下文词汇**');
        blocks.push(
            view.key_vocabulary
                .map(({ word, meaning_in_context }) => `**${markdownText(word)}**：${markdownText(meaning_in_context)}`)
                .join(hardBreak)
        );
    }
    return blocks;
}

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
    const words = visible
        .filter((e) => e.type === 'word')
        .sort((a, b) => {
            const aGroup = letterOf(a.text);
            const bGroup = letterOf(b.text);
            if (aGroup === bGroup) return a.text.localeCompare(b.text);
            if (aGroup === '#') return 1;
            if (bGroup === '#') return -1;
            return aGroup.localeCompare(bGroup);
        });
    const sentences = visible.filter((e) => e.type !== 'word').sort((a, b) => a.id - b.id);
    const sentenceById = new Map(sentences.map((s) => [s.id, s]));

    const out = ['# 我的生词本', ''];
    out.push(`> 导出于 ${stamp(now)} · 单词 ${words.length} · 长难句 ${sentences.length}`, '');

    if (words.length > 0) {
        out.push('## 单词', '');
        let current = '';
        for (const w of words) {
            const letter = letterOf(w.text);
            if (letter !== current) {
                current = letter;
                out.push(`### ${letter === '#' ? '&#35;' : letter}`, '');
            }
            out.push(`**${markdownText(w.text)}**`, '');
            for (const block of resultBlocks(entryDisplay(w))) out.push(block, '');
            const from = sentenceById.get(w.source_id);
            if (from && nonblank(from.text)) out.push('出自：' + markdownText(from.text), '');
        }
    }

    if (sentences.length > 0) {
        out.push('## 长难句', '');
        for (const [index, s] of sentences.entries()) {
            // Navigation never substitutes a shortened/flattened source text.
            out.push(`### ${index + 1}`, '', markdownText(s.text), '');
            for (const block of resultBlocks(entryDisplay(s))) out.push(block, '');
            const children = words.filter((w) => w.source_id === s.id);
            if (children.length > 0) {
                out.push(`生词：${children.map((w) => markdownText(w.text)).join(' · ')}`, '');
            }
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
export const isWord = (text) => {
    const t = text.trim();
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
