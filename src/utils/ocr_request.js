// Every await is a handover point: a newer capture/cancel may have taken ownership.
export function ocrErrorMessage(error, fallback = 'Text recognition failed. Please try again.') {
    const message = typeof error === 'string' ? error : error?.message;
    if (typeof message === 'string' && message.trim()) return message.trim();
    return typeof fallback === 'string' && fallback.trim() ? fallback.trim() : 'Text recognition failed. Please try again.';
}

export async function runOcrRequest({ current, hide, crop, show, recognize, publish, restore, reset, cleanup, noText, failureText, trace = () => {} }) {
    let region;
    let popped = false;
    let stage = 'hide';
    const isCurrent = () => {
        const active = current();
        if (!active) trace('stale-discard', { stage });
        return active;
    };
    try {
        if (!isCurrent()) return;
        await hide();
        trace('overlay-hidden');
        if (!isCurrent()) return;
        stage = 'crop';
        region = await crop();
        trace('cropped');
        if (!isCurrent()) return;
        stage = 'show';
        await show(region);
        trace('result-shown');
        if (!isCurrent()) return;
        popped = true;
        stage = 'recognize';
        const text = await recognize(region.path);
        if (!isCurrent()) return;
        if (typeof text !== 'string' || !text.trim()) throw new Error(noText);
        trace('recognized', { textLength: text.length });
        stage = 'publish';
        await publish(text, false);
        if (isCurrent()) reset();
    } catch (error) {
        if (!isCurrent()) return;
        const message = ocrErrorMessage(error, failureText);
        trace('failed', { stage, errorLength: message.length });
        if (popped) await publish(message, true);
        else await restore(message);
    } finally {
        // Keep the file until the reader has settled, even after cancellation.
        if (region) await cleanup(region.path);
    }
}

// Check native ownership at consumption time as well as at publication time.
// The sequence check also drops checks that resolve after a newer text event.
export function createOcrEventGate(isCurrent) {
    let sequence = 0;
    let latestRequest = 0;
    return {
        invalidate(requestId) {
            // A delayed session announcement must not cancel an already newer text.
            if (requestId !== undefined && requestId <= latestRequest) return false;
            sequence++;
            if (requestId !== undefined) latestRequest = requestId;
            return true;
        },
        async accept(payload, receive) {
            if (typeof payload === 'string') {
                sequence++;
                receive(payload, null);
                return;
            }
            if (!payload || typeof payload.requestId !== 'number' || typeof payload.text !== 'string') return;
            if (payload.requestId < latestRequest) return;
            latestRequest = payload.requestId;
            const own = ++sequence;
            try {
                if (await isCurrent(payload.requestId) && own === sequence) receive(payload.text, payload.requestId);
            } catch { /* A failed ownership check must never resurrect an old result. */ }
        },
    };
}
