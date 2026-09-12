import { extractJson } from '../../../utils/wordbook_format.js';

const object = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);
const strings = (value) => Array.isArray(value) && value.every((item) => typeof item === 'string');

// Validate before rendering, copying, caching or saving. Keep invalid responses verbatim.
export function dictionaryResult(text) {
    const fenced = text.trim().match(/^```(?:json)?\s*([\s\S]*?)\s*```$/i);
    let value;
    try {
        // Parse the whole response first: extracting braces would unwrap [dictionary].
        value = JSON.parse(fenced ? fenced[1] : text);
    } catch {
        value = extractJson(text);
    }
    if (!object(value) || !Array.isArray(value.explanations) || !value.explanations.length) return text;
    if (!value.explanations.every((item) => object(item) && strings(item.explains)
        && (item.trait === undefined || typeof item.trait === 'string'))) return text;
    if (value.pronunciations !== undefined && (!Array.isArray(value.pronunciations)
        || !value.pronunciations.every((item) => object(item) && typeof item.symbol === 'string'))) return text;
    if (value.associations !== undefined && !strings(value.associations)) return text;
    return {
        explanations: value.explanations.map(({ trait, explains }) => ({ trait, explains })),
        pronunciations: (value.pronunciations ?? []).map(({ symbol }) => ({ symbol })),
        associations: value.associations ?? [],
    };
}
