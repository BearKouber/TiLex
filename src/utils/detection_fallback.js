// Detection is advisory; a rejected request must not block the translation queue.
export async function detectionWithFallback(detect, text, from = 'auto') {
    try {
        const detected = await detect(text);
        if (typeof detected !== 'string' || !detected || detected === 'auto') throw new Error('No language detected');
        return { detected, badge: detected };
    } catch {
        return { detected: from, badge: '' };
    }
}
