import assert from 'node:assert/strict';
import { applyTheme } from './theme_subscription.js';
const listeners = new Set();
const media = {
    matches: false,
    addEventListener: (_, fn) => listeners.add(fn),
    removeEventListener: (_, fn) => listeners.delete(fn),
};
let theme;
const setTheme = (next) => {
    theme = next;
};
const matchMedia = () => media;
for (let i = 0; i < 5; i++) {
    const cleanup = applyTheme('system', setTheme, matchMedia);
    assert.equal(listeners.size, 1);
    assert.equal(theme, media.matches ? 'dark' : 'light');
    media.matches = !media.matches;
    listeners.forEach((fn) => fn());
    assert.equal(theme, media.matches ? 'dark' : 'light');
    cleanup();
    applyTheme('light', setTheme, matchMedia);
    media.matches = true;
    listeners.forEach((fn) => fn());
    assert.equal(theme, 'light');
    assert.equal(listeners.size, 0);
}
console.log('theme subscription: manual selection and listener cleanup passed');
