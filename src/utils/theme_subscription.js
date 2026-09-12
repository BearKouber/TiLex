export function applyTheme(theme, setTheme, matchMedia) {
    if (!theme) return;
    if (theme !== 'system') {
        setTheme(theme);
        return;
    }
    const media = matchMedia('(prefers-color-scheme: dark)');
    const changed = () => setTheme(media.matches ? 'dark' : 'light');
    changed();
    media.addEventListener('change', changed);
    return () => media.removeEventListener('change', changed);
}
