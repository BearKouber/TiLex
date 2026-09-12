// Every await is a handover point: a newer capture/cancel may have taken ownership.
export async function runOcrRequest({ current, hide, crop, show, recognize, publish, restore, reset, cleanup, noText }) {
    let region;
    let popped = false;
    try {
        if (!current()) return;
        await hide();
        if (!current()) return;
        region = await crop();
        if (!current()) return;
        await show(region);
        if (!current()) return;
        popped = true;
        const text = await recognize(region.path);
        if (!current()) return;
        if (!text) throw new Error(noText);
        await publish(text, false);
        if (current()) reset();
    } catch (error) {
        if (!current()) return;
        const message = error?.message ?? String(error);
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
                receive(payload);
                return;
            }
            if (!payload || typeof payload.requestId !== 'number' || typeof payload.text !== 'string') return;
            if (payload.requestId < latestRequest) return;
            latestRequest = payload.requestId;
            const own = ++sequence;
            try {
                if (await isCurrent(payload.requestId) && own === sequence) receive(payload.text);
            } catch { /* A failed ownership check must never resurrect an old result. */ }
        },
    };
}
