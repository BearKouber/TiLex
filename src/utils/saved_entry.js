import { normalizeResult } from './translation_result.js';

const isRecord = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);

function structuredText(value) {
    const base = value.kind === 'sentence'
        ? [value.translation]
        : [
              ...value.pronunciations.map((p) => p.symbol),
              ...value.explanations.map((e) => `${e.trait ?? ''} ${e.explains.join(', ')}`.trim()),
              ...value.associations,
          ];
    return [
        ...base,
        ...(value.examples ?? []).flatMap((example) => [example.text, example.translation]),
        ...(value.notes ?? []),
    ].filter(Boolean).join('\n');
}

// Only validated structures reach copy/search/save. Invalid AI responses already
// arrive as their original strings from the service boundary.
export function resultText(value) {
    if (typeof value === 'string') return value.trim();
    const result = normalizeResult(value);
    return result ? structuredText(result) : '';
}

function candidate(item) {
    if (!item || item.error) return null;
    const detail = normalizeResult(item.result);
    const translation = detail ? structuredText(detail) : resultText(item.result);
    if (!translation) return null;
    // Service identity comes from the request, never a model-supplied field.
    const service = typeof item.key === 'string'
        ? item.key.split('@')[0]
        : item.serviceName ?? item.meta?.serviceName;
    const supplementedAi = service === 'ai' && detail?.schemaVersion === 1 &&
        (detail.kind === 'word' || detail.kind === 'sentence') &&
        (detail.examples.length > 0 || detail.notes.length > 0);
    const dictionary = detail && detail.kind !== 'sentence';
    return { translation, detail, priority: supplementedAi ? 0 : dictionary ? 1 : 2 };
}

// Items stay in the service order captured when this request began. Equal
// priorities keep the first candidate, regardless of response completion order.
export function entrySnapshot(text, items) {
    let selected = null;
    for (const item of items) {
        const next = candidate(item);
        if (next && (!selected || next.priority < selected.priority)) selected = next;
    }
    return {
        text,
        translation: selected?.translation ?? '',
        detail: selected?.detail ?? null,
    };
}

// Each selection owns its entry and write queue, even after the popup moves on.
export function createSavedEntry(text, { add, update, onStatus }) {
    let items = [];
    const serviceOrder = new Map();
    let requested = false;
    let entryId = null;
    let queue = Promise.resolve();
    const persist = () => {
        onStatus('saving');
        queue = queue.then(async () => {
            const snapshot = entrySnapshot(text, items);
            if (entryId === null) entryId = await add(snapshot);
            else await update(entryId, snapshot);
            onStatus('ok');
        }).catch((error) => {
            console.error(error);
            onStatus('error');
        });
        return queue;
    };
    return {
        text,
        get requested() { return requested; },
        setItems(next) {
            items = next.map((item) => {
                if (!serviceOrder.has(item.key)) serviceOrder.set(item.key, serviceOrder.size);
                return { ...item };
            }).sort((a, b) => serviceOrder.get(a.key) - serviceOrder.get(b.key));
            return requested ? persist() : queue;
        },
        patch(key, fields) {
            items = items.map((item) => item.key === key ? { ...item, ...fields, key: item.key } : item);
            return requested ? persist() : queue;
        },
        save() { requested = true; return persist(); },
    };
}

function legacyAnalysis(detail) {
    const syntax = detail?.syntax_breakdown;
    const main = typeof syntax?.main_clause === 'string' ? syntax.main_clause : '';
    const modifiers = typeof syntax?.clauses_and_modifiers === 'string' ? syntax.clauses_and_modifiers : '';
    return {
        syntax_breakdown: isRecord(syntax) && (main.trim() || modifiers.trim())
            ? { main_clause: main, clauses_and_modifiers: modifiers }
            : null,
        nuance_note: typeof detail?.nuance_note === 'string' ? detail.nuance_note : '',
        key_vocabulary: Array.isArray(detail?.key_vocabulary)
            ? detail.key_vocabulary.filter((value) => isRecord(value) &&
                typeof value.word === 'string' && value.word.trim() &&
                typeof value.meaning_in_context === 'string' && value.meaning_in_context.trim())
                .map(({ word, meaning_in_context }) => ({ word, meaning_in_context }))
            : [],
    };
}

// A single safe view for popup, wordbook and export. The searchable translation
// column is flattened; structured detail supplies each visible section once.
export function entryDisplay(entry) {
    const raw = isRecord(entry?.detail) ? entry.detail : null;
    const detail = normalizeResult(raw);
    const translation = typeof entry?.translation === 'string' ? entry.translation : '';
    const kind = detail ? detail.kind ?? 'word' : 'text';
    let associations = detail?.associations ?? [];
    // Older saves omitted associations but retained them after the flattened dictionary.
    if (kind === 'word' && !raw.kind && raw.associations == null) {
        const prefix = structuredText({ ...detail, associations: [] });
        if (translation.startsWith(prefix + '\n')) {
            associations = translation.slice(prefix.length + 1).split('\n').filter((line) => line.trim());
        }
    }
    return {
        kind,
        translation: kind === 'word' ? '' : kind === 'sentence' ? detail.translation : translation,
        pronunciations: detail?.pronunciations ?? [],
        explanations: detail?.explanations ?? [],
        associations,
        examples: detail?.examples ?? [],
        notes: detail?.notes ?? [],
        ...legacyAnalysis(raw),
    };
}
