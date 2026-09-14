const object = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);
const strings = (value) => Array.isArray(value) && value.every((item) => typeof item === 'string');
const nonblank = (value) => typeof value === 'string' && value.trim().length > 0;

// Sentence categories picked by the model from the source language. Insertion
// order is the export group order; validation, export and Wordbook share it.
export const SENTENCE_CATEGORIES = Object.freeze({
    EN01: { lang: '英文', no: '01', name: '定语从句类' },
    EN02: { lang: '英文', no: '02', name: '状语从句类' },
    EN03: { lang: '英文', no: '03', name: '名词性从句类' },
    EN04: { lang: '英文', no: '04', name: '非谓语与独立主格' },
    EN05: { lang: '英文', no: '05', name: '特殊句式（倒装/强调）' },
    EN06: { lang: '英文', no: '06', name: '并列与长插入语' },
    ZH01: { lang: '中文', no: '01', name: '多重长定语类' },
    ZH02: { lang: '中文', no: '02', name: '长状语阻隔类' },
    ZH03: { lang: '中文', no: '03', name: '多层复句嵌套类' },
    ZH04: { lang: '中文', no: '04', name: '流水句与主语隐换类' },
    ZH05: { lang: '中文', no: '05', name: '长连谓与兼语句' },
});

// Shared boundary for service responses, saved records and their display projections.
// A legacy dictionary deliberately has no kind/version: only explicit new results
// may receive the higher-priority AI supplements treatment when saving.
export function normalizeResult(value, expectedKind) {
    if (!object(value)) return null;
    const kind = value.kind;
    if (kind !== undefined && kind !== 'word' && kind !== 'sentence') return null;
    if (expectedKind !== undefined && (kind ?? 'word') !== expectedKind) return null;

    if (
        value.pronunciations !== undefined &&
        (!Array.isArray(value.pronunciations) ||
            !value.pronunciations.every((item) => object(item) && typeof item.symbol === 'string'))
    )
        return null;
    if (
        value.explanations !== undefined &&
        (!Array.isArray(value.explanations) ||
            !value.explanations.every(
                (item) =>
                    object(item) &&
                    strings(item.explains) &&
                    (item.trait === undefined || typeof item.trait === 'string')
            ))
    )
        return null;
    if (value.associations !== undefined && !strings(value.associations)) return null;
    if (value.notes !== undefined && !strings(value.notes)) return null;
    if (
        value.examples !== undefined &&
        (!Array.isArray(value.examples) ||
            !value.examples.every((item) => object(item) && nonblank(item.text) && nonblank(item.translation)))
    )
        return null;
    if (value.translation !== undefined && typeof value.translation !== 'string') return null;
    if (
        value.syntax_breakdown !== undefined &&
        value.syntax_breakdown !== null &&
        (!object(value.syntax_breakdown) ||
            (value.syntax_breakdown.main_clause !== undefined && typeof value.syntax_breakdown.main_clause !== 'string') ||
            (value.syntax_breakdown.clauses_and_modifiers !== undefined &&
                typeof value.syntax_breakdown.clauses_and_modifiers !== 'string'))
    )
        return null;
    if (value.nuance_note !== undefined && value.nuance_note !== null && typeof value.nuance_note !== 'string')
        return null;
    if (
        value.key_vocabulary !== undefined &&
        value.key_vocabulary !== null &&
        (!Array.isArray(value.key_vocabulary) ||
            !value.key_vocabulary.every(
                (item) =>
                    object(item) &&
                    typeof item.word === 'string' &&
                    typeof item.meaning_in_context === 'string'
            ))
    )
        return null;

    const supplements = {
        examples: (value.examples ?? []).map(({ text, translation }) => ({ text, translation })),
        notes: (value.notes ?? []).filter(nonblank),
    };
    if (kind === 'sentence') {
        if (!nonblank(value.translation)) return null;
        const result = { schemaVersion: 1, kind, translation: value.translation, ...supplements };
        // Tags are secondary: an invalid one is dropped alone, never the translation.
        const category = typeof value.category === 'string' ? value.category.trim().toUpperCase() : '';
        if (Object.hasOwn(SENTENCE_CATEGORIES, category)) result.category = category;
        const difficulty = ['number', 'string'].includes(typeof value.difficulty) ? Number(value.difficulty) : NaN;
        if ([1, 2, 3].includes(difficulty)) {
            result.difficulty = difficulty;
            if (nonblank(value.difficulty_reason)) result.difficulty_reason = value.difficulty_reason.trim();
        }
        const syntax = value.syntax_breakdown;
        if (object(syntax) && (nonblank(syntax.main_clause) || nonblank(syntax.clauses_and_modifiers))) {
            result.syntax_breakdown = {
                main_clause: nonblank(syntax.main_clause) ? syntax.main_clause.trim() : '',
                clauses_and_modifiers: nonblank(syntax.clauses_and_modifiers)
                    ? syntax.clauses_and_modifiers.trim()
                    : '',
            };
        }
        if (nonblank(value.nuance_note)) {
            result.nuance_note = value.nuance_note.trim();
        }
        if (Array.isArray(value.key_vocabulary)) {
            const vocab = value.key_vocabulary
                .filter((item) => object(item) && nonblank(item.word) && nonblank(item.meaning_in_context))
                .map(({ word, meaning_in_context }) => ({
                    word: word.trim(),
                    meaning_in_context: meaning_in_context.trim(),
                }));
            if (vocab.length > 0) {
                result.key_vocabulary = vocab;
            }
        }
        return result;
    }

    const explanations = (value.explanations ?? [])
        .map(({ trait, explains }) => ({ trait, explains: explains.filter(nonblank) }))
        .filter(({ explains }) => explains.length > 0);
    if (!explanations.length) return null;
    const dictionary = {
        pronunciations: (value.pronunciations ?? [])
            .filter(({ symbol }) => nonblank(symbol))
            .map(({ symbol }) => ({ symbol })),
        explanations,
        associations: (value.associations ?? []).filter(nonblank),
    };
    if (kind === 'word') return { schemaVersion: 1, kind, ...dictionary, ...supplements };
    return {
        ...dictionary,
        ...(value.examples !== undefined ? { examples: supplements.examples } : {}),
        ...(value.notes !== undefined ? { notes: supplements.notes } : {}),
    };
}
