// Markdown 导出（个人需求 PRD §3）。纯字符串拼接，不引 markdown 库。
//
// 目标是「看着像一份真的笔记」：一个词一行，不是一个词一屏。
//
// 2026-09-08 重写，三件事（实测反馈，见 PROGRESS §7.28）：
//
// 1. **不再写 <a id> 锚点。** 用户的编辑器把它当纯文本显示出来，
//    满屏的 `<a id="word-2"></a>` 比它能带来的跳转价值大得多。
//    导航改靠标题（编辑器的大纲面板天然就是目录），
//    词↔句的关系改成**直接把原句写在词底下**，不跳了。
// 2. **单词从 #### 标题改成列表项。** 标题 + 空行 + 分隔线是
//    「一个词一屏」的根源；顺带大纲里不再被每个单词刷屏，只剩字母。
// 3. **有 detail 的词不再重复输出 translation。** 这是个真 bug：
//    PopResult 存的 translation 就是 plainText(词典结果)，和 detail 同源，
//    两边都印就是同一堆音标释义连着出两遍。

// A–Z 分组；数字、中文、符号开头的统统进 # 桶。
const letterOf = (text) => {
    const c = (text ?? '').trim().charAt(0).toUpperCase();
    return c >= 'A' && c <= 'Z' ? c : '#';
};

const pad2 = (n) => String(n).padStart(2, '0');
const stamp = (d) =>
    `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;

// 译文可能是 plainText 拼出来的多行，放进列表项得压成一行。
const oneLine = (t) => (t ?? '').replace(/\s*\n\s*/g, ' ').trim();

// 一个词一行：**词** `音标` — *词性* 释义；*词性* 释义
function wordLine(w) {
    const parts = [`**${w.text}**`];

    const symbols = (w.detail?.pronunciations ?? []).map((p) => p.symbol).filter(Boolean);
    if (symbols.length > 0) parts.push('`' + symbols.join(' ') + '`');

    const senses = (w.detail?.explanations ?? [])
        .map((item) => {
            const explains = (item.explains ?? []).filter(Boolean).join(', ');
            if (!explains && !item.trait) return '';
            return item.trait ? `*${item.trait}* ${explains}`.trim() : explains;
        })
        .filter(Boolean);

    if (senses.length > 0) {
        parts.push('— ' + senses.join('；'));
    } else if (w.translation) {
        // 没有词典结果时 translation 才是唯一的释义来源。
        parts.push('— ' + oneLine(w.translation));
    }
    return '- ' + parts.join(' ');
}

// entries：已经 parseDetail 过的数组。now 可注入，测试才能有稳定输出。
export function buildMarkdown(entries, now = new Date()) {
    const words = entries.filter((e) => e.type === 'word').sort((a, b) => a.text.localeCompare(b.text));
    const sentences = entries.filter((e) => e.type !== 'word').sort((a, b) => a.id - b.id);
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
                // 上一组的最后一个列表项和这个标题之间必须空一行，
                // 否则严格一点的解析器会把 ### 并进列表项里。
                if (out[out.length - 1] !== '') out.push('');
                out.push(`### ${letter}`, '');
            }
            out.push(wordLine(w));

            const associations = (w.detail?.associations ?? []).filter(Boolean);
            if (associations.length > 0) out.push(`  - 搭配：${associations.join(' · ')}`);

            // 原句直接写在这里，代替原来那个靠锚点的「查看原句」链接。
            const from = w.source_id && sentenceById.get(w.source_id);
            if (from) out.push(`  - 出自：${oneLine(from.text)}`);
        }
        out.push('');
    }

    if (sentences.length > 0) {
        out.push('## 长难句', '');
        for (const s of sentences) {
            out.push(`### ${oneLine(s.text)}`, '');
            if (s.translation) out.push(oneLine(s.translation), '');

            const children = words.filter((w) => w.source_id === s.id);
            if (children.length > 0) {
                out.push(`生词：${children.map((w) => w.text).join(' · ')}`, '');
            }

            const syntax = s.detail?.syntax_breakdown;
            if (syntax?.main_clause || syntax?.clauses_and_modifiers || s.detail?.nuance_note) {
                // <details> 是块级元素，绝大多数编辑器会当 HTML 渲染，
                // 和上面删掉的行内 <a id> 不是一回事。
                out.push('<details>', '<summary>语法拆解</summary>', '');
                if (syntax?.main_clause) out.push(`**主干**：${syntax.main_clause}`, '');
                if (syntax?.clauses_and_modifiers) out.push(`**修饰**：${syntax.clauses_and_modifiers}`, '');
                if (s.detail?.nuance_note) out.push(`> ${s.detail.nuance_note}`, '');
                out.push('</details>', '');
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
    (detail?.explanations ?? [])
        .map((e) => (e.explains ?? []).join(', '))
        .filter(Boolean)
        .join('; ');

// 列表过滤：type 筛选 + 原文/译文/释义三处一起搜，关键词已经 trim + 小写。
export const matchEntry = (entry, filter, keyword) => {
    if (filter !== 'all' && entry.type !== filter) return false;
    if (!keyword) return true;
    return `${entry.text} ${entry.translation ?? ''} ${wordSummary(entry.detail)}`.toLowerCase().includes(keyword);
};
