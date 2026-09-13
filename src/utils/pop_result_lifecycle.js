// One blur timer per displayed generation. Both the timer and its asynchronous
// focus reply must still belong to that generation before they can hide it.
export function createBlurGuard({
    isFocused,
    hide,
    isPinned = () => false,
    trace = () => {},
    now = Date.now,
    schedule = setTimeout,
    cancel = clearTimeout,
}) {
    let generation = 0;
    let shownAt = 0;
    let timer;
    let context = {};
    const invalidate = () => {
        generation++;
        if (timer !== undefined) cancel(timer);
        timer = undefined;
    };
    const dismiss = (source) => {
        invalidate();
        trace('hide', { ...context, source });
        void hide();
    };
    return {
        begin(runId, requestId) {
            invalidate();
            shownAt = now();
            context = { runId, requestId };
        },
        invalidate,
        dismiss,
        focus() {
            // A new focus event supersedes every earlier blur, including a
            // focus query already in flight for this same displayed result.
            invalidate();
            trace('focus-observed', context);
        },
        blur() {
            if (isPinned()) {
                trace('blur-ignored-pinned', context);
                return;
            }
            const own = generation;
            const remaining = 300 - (now() - shownAt);
            trace('blur', { ...context, grace: remaining >= 0 });
            if (remaining < 0) {
                dismiss('blur');
                return;
            }
            if (timer !== undefined) cancel(timer);
            timer = schedule(
                async () => {
                    timer = undefined;
                    if (own !== generation) return;
                    if (isPinned()) return;
                    try {
                        const focused = await isFocused();
                        if (own !== generation) return;
                        if (isPinned()) return;
                        trace('focus-check', { ...context, focused });
                        if (!focused) dismiss('delayed-blur');
                    } catch {
                        // An unavailable focus state is not evidence of a real blur.
                        if (own === generation) trace('focus-check-failed', context);
                    }
                },
                Math.max(0, remaining)
            );
        },
    };
}
