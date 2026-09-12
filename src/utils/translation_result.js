const object = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);
const strings = (value) => Array.isArray(value) && value.every((item) => typeof item === 'string');
const nonblank = (value) => typeof value === 'string' && value.trim().length > 0;

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

    const supplements = {
        examples: (value.examples ?? []).map(({ text, translation }) => ({ text, translation })),
        notes: (value.notes ?? []).filter(nonblank),
    };
    if (kind === 'sentence') {
        if (!nonblank(value.translation)) return null;
        return { schemaVersion: 1, kind, translation: value.translation, ...supplements };
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
