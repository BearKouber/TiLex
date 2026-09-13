import { matchEntry } from './wordbook_format.js';

export function visibleWordbookEntries({ entries, keyword, filter }) {
    const kw = keyword.trim().toLowerCase();
    return entries.filter((entry) => matchEntry(entry, filter, kw));
}

// Run against the latest view after SQLite succeeds, not the view at click time.
export function removeFromWordbook(view, ids) {
    const removed = new Set(ids);
    const list = visibleWordbookEntries(view);
    const selected = list.find((entry) => entry.id === view.selectedId) ?? list[0];
    let next = selected;
    if (selected && removed.has(selected.id)) {
        const index = list.indexOf(selected);
        next = list.slice(index + 1).find((entry) => !removed.has(entry.id));
        if (!next) {
            next = list
                .slice(0, index)
                .reverse()
                .find((entry) => !removed.has(entry.id));
        }
    }
    return {
        ...view,
        entries: view.entries.filter((entry) => !removed.has(entry.id)),
        selectedId: next?.id ?? null,
    };
}

export async function softDeleteWordbookEntries(db, ids) {
    const uniqueIds = [...new Set(ids)];
    if (uniqueIds.length === 0) return;
    // One statement is atomic: a failed batch cannot leave a partially deleted list.
    const placeholders = uniqueIds.map((_, index) => `$${index + 1}`).join(', ');
    await db.execute(`UPDATE entries SET deleted=1 WHERE id IN (${placeholders})`, uniqueIds);
}
