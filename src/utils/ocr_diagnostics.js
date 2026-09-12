import { info } from 'tauri-plugin-log-api';

// Callers supply fixed event names and numeric/state fields only. Filter fields
// here so later diagnostics cannot accidentally persist a response or selection.
const FIELDS = new Set(['requestId', 'runId', 'textLength', 'errorLength', 'stage', 'source', 'focused', 'grace']);
export function traceOcr(event, details = {}) {
    const fields = Object.fromEntries(Object.entries(details).filter(([key]) => FIELDS.has(key)));
    void info(`OCR lifecycle: ${event} ${JSON.stringify({ at: Date.now(), ...fields })}`).catch(() => {});
}
