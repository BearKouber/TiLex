export const resultText = (value) =>
    typeof value === 'string'
        ? value.trim()
        : [
              ...(value?.pronunciations ?? []).map((p) => p.symbol).filter(Boolean),
              ...(value?.explanations ?? []).map((e) => `${e.trait ?? ''} ${(e.explains ?? []).join(', ')}`.trim()),
              ...(value?.associations ?? []),
          ].join('\n');

export function entrySnapshot(text, items) {
    const results = items.filter((item) => !item.error).map((item) => item.result);
    const dictionary = results.find((value) => value && typeof value === 'object');
    return {
        text,
        translation: dictionary ? resultText(dictionary) : results.map(resultText).find(Boolean) ?? '',
        detail: dictionary ? {
            pronunciations: dictionary.pronunciations ?? [],
            explanations: dictionary.explanations ?? [],
            associations: dictionary.associations ?? [],
        } : null,
    };
}

// Each selection owns its entry and write queue, even after the popup moves on.
export function createSavedEntry(text, { add, update, onStatus }) {
    let items = [];
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
        setItems(next) { items = next; if (requested) persist(); },
        patch(key, fields) {
            items = items.map((item) => item.key === key ? { ...item, ...fields } : item);
            if (requested) persist();
        },
        save() { requested = true; return persist(); },
    };
}

export function entryDisplay(entry) {
    const detail = entry.detail;
    const hasDictionary = (detail?.explanations ?? []).some((e) => e.explains?.length);
    let associations = detail?.associations ?? [];
    // Older saves omitted associations but retained them after the flattened dictionary.
    if (hasDictionary && detail.associations == null) {
        const prefix = resultText({ ...detail, associations: [] });
        if (entry.translation?.startsWith(prefix + '\n')) {
            associations = entry.translation.slice(prefix.length + 1).split('\n').filter(Boolean);
        }
    }
    return { translation: hasDictionary ? '' : entry.translation, associations };
}
