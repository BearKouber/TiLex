import { effectiveAiConfig } from '../services/translate/ai/instructions.js';

// LRU cache for translation results, so repeating a selection skips the network.
// A Map preserves insertion order, so the oldest entry is simply the first key.
const MAX_ENTRIES = 200;
const cache = new Map();

function canonical(value) {
    if (Array.isArray(value)) return value.map(canonical);
    if (value !== null && typeof value === 'object') {
        return Object.fromEntries(Object.keys(value).sort().map((key) => [key, canonical(value[key])]));
    }
    return value;
}

// Configuration may contain credentials: these keys stay in memory and must never be logged.
export const cacheKey = (text, from, to, service, config = {}, detected = '') =>
    JSON.stringify([text, from, to, service, canonical(config), detected]);

// AI requests and cache identity use the same effective configuration. Appearance
// and migration backups stay in the settings draft, outside this request snapshot.
export const requestConfigSnapshot = (config, service = '') => JSON.parse(JSON.stringify(
    service.split('@')[0] === 'ai' ? effectiveAiConfig(config) : config
));

export function getCached(key) {
    if (!cache.has(key)) return undefined;
    const value = cache.get(key);
    cache.delete(key);
    cache.set(key, value); // move to newest
    return value;
}

export function setCached(key, value) {
    cache.delete(key);
    cache.set(key, value);
    if (cache.size > MAX_ENTRIES) cache.delete(cache.keys().next().value);
}
