// LRU cache for translation results, so repeating a selection skips the network.
// A Map preserves insertion order, so the oldest entry is simply the first key.
const MAX_ENTRIES = 200;
const cache = new Map();

export const cacheKey = (text, from, to, service) => JSON.stringify([text, from, to, service]);

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
