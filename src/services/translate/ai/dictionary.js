import { extractJson } from '../../../utils/wordbook_format.js';
import { normalizeResult } from '../../../utils/translation_result.js';

// Validate before rendering, copying, caching or saving. Keep invalid responses verbatim.
export function dictionaryResult(text, expectedKind = 'word') {
    const fenced = text.trim().match(/^```(?:json)?\s*([\s\S]*?)\s*```$/i);
    let value;
    try {
        // Parse the whole response first: extracting braces would unwrap [dictionary].
        value = JSON.parse(fenced ? fenced[1] : text);
    } catch {
        value = extractJson(text);
    }
    return normalizeResult(value, expectedKind) ?? text;
}
