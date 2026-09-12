// One coordinator per webview; native generations also reject work queued in
// other windows before a delete/reload. The native store owns persisted state.
export function createConfigCoordinator({
    commit,
    read,
    onError = () => {},
    delay = 500,
    schedule = setTimeout,
    unschedule = clearTimeout,
}) {
    let state = { values: {}, generations: {}, deleted: [], revision: -1 };
    const pending = new Map();
    const inFlight = new Map();
    const listeners = new Map();
    let queue = Promise.resolve();
    const generation = (key) => state.generations[key] ?? 0;
    const value = (key) => {
        if (pending.has(key)) return pending.get(key).value;
        if (inFlight.has(key)) return inFlight.get(key).value;
        return state.values[key];
    };
    const notify = (key) => listeners.get(key)?.forEach((fn) => fn(value(key)));
    const cancel = (key) => {
        // The IPC may already be running; retire its local draft immediately.
        // Native generations still protect against stale queued writes.
        inFlight.delete(key);
        const task = pending.get(key);
        if (task) {
            unschedule(task.timer);
            pending.delete(key);
            task.resolve(false);
        }
    };
    const accept = (next) => {
        if (next.revision <= state.revision) return;
        for (const [key, task] of pending) {
            if (task.generation !== (next.generations[key] ?? 0) || next.deleted.includes(key)) cancel(key);
        }
        for (const [key, task] of inFlight) {
            if (task.generation !== (next.generations[key] ?? 0) || next.deleted.includes(key)) inFlight.delete(key);
        }
        state = next;
        for (const key of listeners.keys()) notify(key);
    };
    const refresh = async () => {
        accept(await read());
        return state;
    };
    const submit = (operations) => {
        // Capture generations at the intent boundary, not after queue delay.
        const prepared = operations.map((op) => ({ generation: generation(op.key), ...op }));
        const job = queue.then(async () => {
            try {
                accept(await commit(prepared));
                return state;
            } catch (error) {
                try {
                    await refresh();
                } catch {
                    /* Preserve the last committed snapshot. */
                }
                for (const op of prepared) notify(op.key);
                throw error;
            }
        });
        queue = job.catch(() => {});
        return job;
    };
    const dispatchSet = async (key, task) => {
        // Keep the latest intent visible while its commit is queued or awaiting
        // IPC, so a subsequent deletion retains an already-dispatched reorder.
        inFlight.set(key, task);
        notify(key);
        try {
            return await submit([{ kind: 'set', key, value: task.value, generation: task.generation }]);
        } finally {
            if (inFlight.get(key) === task) inFlight.delete(key);
            notify(key);
        }
    };
    const set = (key, next, immediate = false) => {
        cancel(key);
        if (immediate) return dispatchSet(key, { value: next, generation: generation(key) });
        let resolve;
        const done = new Promise((r) => {
            resolve = r;
        });
        const task = { value: next, generation: generation(key), resolve };
        task.timer = schedule(async () => {
            if (pending.get(key) !== task) return;
            pending.delete(key);
            try {
                await dispatchSet(key, task);
                resolve(true);
            } catch (error) {
                onError(error);
                notify(key);
                resolve(false);
            }
        }, delay);
        pending.set(key, task);
        notify(key);
        return done;
    };
    const batch = (operations) => {
        for (const op of operations) cancel(op.key);
        return submit(operations);
    };
    return {
        accept,
        refresh,
        set,
        batch,
        cancel,
        value,
        generation,
        subscribe(key, fn) {
            if (!listeners.has(key)) listeners.set(key, new Set());
            listeners.get(key).add(fn);
            return () => {
                listeners.get(key)?.delete(fn);
            };
        },
        async initialize(key, fallback, persist = true) {
            if (value(key) !== undefined) return value(key);
            if (persist && !state.deleted.includes(key)) {
                await submit([{ kind: 'setIfAbsent', key, value: fallback }]);
            }
            return value(key) ?? fallback;
        },
        async removeService(listKey, key, name = key) {
            const list = (value(listKey) ?? []).filter((item) => item !== name);
            await batch([
                { kind: 'set', key: listKey, value: list, invalidate: true },
                { kind: 'delete', key },
            ]);
        },
        async saveService(listKey, key, config, name = key, adding = false) {
            const list = value(listKey) ?? [];
            if (!adding && !list.includes(name)) throw new Error('config.removed');
            await batch([
                { kind: 'set', key, value: config, revive: adding },
                { kind: 'set', key: listKey, value: list.includes(name) ? list : [...list, name], invalidate: adding },
            ]);
        },
    };
}
