import { fetch, Body } from '@tauri-apps/api/http';
import { formatOf } from './protocol';

// 模型测速的队列和结果都放在组件外面：弹窗关掉之后测速还得接着跑完，
// 已经测出来的下次打开要还在（用户明确要的两条）。
//
// 结果按端点地址存，两个实例指向同一家服务就共用一份。
// localStorage 存的是 { [endpoint]: { models: [...], latency: { [model]: 毫秒 | 'failed' } } }
const STORE_KEY = 'openai_model_latency';
// 一个一个测。并发打出去反代会限流，返回的 429 和「这个模型真的不能用」
// 长得一模一样，测出来一片红色的失败，全是假的。
const CONCURRENCY = 1;
// 毫秒。两次探测之间喘口气，同样是为了不触发限流
const GAP = 300;
// 秒。不设的话挂住的模型会一直排在队列里
const TIMEOUT = 15;
// 限流、网关抽风、超时 —— 这些不是模型的问题，等一下重来
const RETRY_WAIT = 2000;
const isTransient = (status) => status === 0 || status === 429 || status >= 500;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const HELLO = [{ role: 'user', content: 'Hello' }];

const read = () => {
    try {
        return JSON.parse(localStorage.getItem(STORE_KEY)) ?? {};
    } catch {
        return {};
    }
};

let cache = read();
const listeners = new Set();
// endpoint -> Set(排队中或正在测的 model)
const running = new Map();
// endpoint -> { list: [待测 model], active: 并发数, apiKey }
const queues = new Map();

const notify = () => listeners.forEach((fn) => fn());

const persist = () => {
    try {
        localStorage.setItem(STORE_KEY, JSON.stringify(cache));
    } catch {
        // 隐私模式之类写不进去，内存里那份照样能用
    }
};

export function subscribe(fn) {
    listeners.add(fn);
    return () => listeners.delete(fn);
}

// 缓存按端点存，两个实例指向同一家服务就共用一份。
// 用 modelsUrl 而不是 chatUrl 当 key：Google 那家把模型名塞在 chat 地址里，
// 用 chatUrl 的话每个模型都会开一份缓存。modelsUrl 四家都和模型无关。
export const endpointOf = (requestPath, apiFormat) => {
    try {
        return formatOf(apiFormat).modelsUrl(requestPath);
    } catch {
        // 地址还没填全的时候 new URL 会抛
        return '';
    }
};

// 之前拉到过的模型列表。没有就返回空数组，调用方自己兜底。
export const getModels = (requestPath, apiFormat) => cache[endpointOf(requestPath, apiFormat)]?.models ?? [];

// 毫秒数 | 'failed' | 'testing' | undefined（没测过）
export function getLatency(requestPath, model, apiFormat) {
    const endpoint = endpointOf(requestPath, apiFormat);
    if (running.get(endpoint)?.has(model)) return 'testing';
    return cache[endpoint]?.latency?.[model];
}

export function getAllLatencies(requestPath, apiFormat) {
    const endpoint = endpointOf(requestPath, apiFormat);
    return cache[endpoint]?.latency ?? {};
}

// 获取测速进度与当前状态
export function getBenchmarkProgress(requestPath, apiFormat) {
    const endpoint = endpointOf(requestPath, apiFormat);
    const queue = queues.get(endpoint);
    const busy = running.get(endpoint);
    const isRunning = (busy && busy.size > 0) || (queue && queue.list.length > 0) || (queue && !!queue.currentModel);
    return {
        isRunning: !!isRunning,
        currentModel: queue?.currentModel || (busy && busy.size > 0 ? [...busy][0] : null),
        total: queue?.total || 0,
        completed: queue?.completed || 0,
        remaining: queue?.list?.length || 0,
    };
}

export function setModels(requestPath, models, apiFormat) {
    const endpoint = endpointOf(requestPath, apiFormat);
    cache[endpoint] = { models, latency: cache[endpoint]?.latency ?? {} };
    persist();
    notify();
}

const record = (endpoint, model, value) => {
    const entry = cache[endpoint] ?? { models: [], latency: {} };
    cache[endpoint] = { ...entry, latency: { ...entry.latency, [model]: value } };
    persist();
};

// 极短的一次 chat 请求，掐往返耗时。
// 成功返回毫秒数，失败返回 HTTP 状态码（网络层错误 / 超时算 0）。
async function probeOnce(url, fmt, apiKey, body) {
    const start = performance.now();
    try {
        const res = await fetch(url, {
            method: 'POST',
            headers: { 'Content-Type': 'application/json', ...fmt.headers(apiKey) },
            timeout: TIMEOUT,
            body: Body.json(body),
        });
        return res.ok ? { ms: Math.round(performance.now() - start) } : { status: res.status };
    } catch {
        // 非 2xx 是 res.ok === false，不抛；网络层错误才抛，两种都接住
        return { status: 0 };
    }
}

// 一次探测最多打两枪，第二枪的形态取决于第一枪怎么死的：
// 被限流 / 网关抽风 → 等一下原样重来；
// 被拒绝 → 去掉 max_tokens 再试（不少网关对新模型只认 max_completion_tokens，
// 或者嫌 5 太小直接 400，这跟模型能不能用是两回事）。
async function probe(requestPath, fmt, apiKey, model) {
    const url = fmt.chatUrl(requestPath, model);
    const short = fmt.body(model, HELLO, { max_tokens: 5 });
    const first = await probeOnce(url, fmt, apiKey, short);
    if (first.ms !== undefined) return first.ms;

    if (isTransient(first.status)) {
        await sleep(RETRY_WAIT);
        const retry = await probeOnce(url, fmt, apiKey, short);
        return retry.ms ?? 'failed';
    }
    const bare = await probeOnce(url, fmt, apiKey, fmt.body(model, HELLO, {}));
    return bare.ms ?? 'failed';
}

async function worker(endpoint, queue) {
    while (queue.list.length > 0) {
        const model = queue.list.shift();
        queue.currentModel = model;
        notify();

        const result = await probe(queue.requestPath, queue.fmt, queue.apiKey, model);
        record(endpoint, model, result);
        running.get(endpoint)?.delete(model);
        queue.completed = (queue.completed || 0) + 1;
        queue.currentModel = null;
        notify();

        if (queue.list.length > 0) await sleep(GAP);
    }
    queue.active -= 1;
    queue.currentModel = null;
    notify();
}

// 停止当前端点的测速队列。只丢「还在排队的」——正在飞的那一发拦不住，
// 得让它留在 busy 里由 worker 自己收尾，否则马上再点开始会把它重复测一遍
// （两次结果还会互相覆盖）。currentModel 同理，由 worker 清。
export function stopBenchmark(requestPath, apiFormat) {
    const endpoint = endpointOf(requestPath, apiFormat);
    if (!endpoint) return;
    const queue = queues.get(endpoint);
    const busy = running.get(endpoint);
    if (busy) {
        busy.clear();
        if (queue?.currentModel) busy.add(queue.currentModel);
    }
    if (queue) queue.list = [];
    notify();
}

// 触发测速。支持 force 强制重测所有模型
export function measure(requestPath, apiKey, models, apiFormat, force = false) {
    const endpoint = endpointOf(requestPath, apiFormat);
    if (!endpoint || !models || models.length === 0) return;
    const busy = running.get(endpoint) ?? new Set();
    running.set(endpoint, busy);

    if (force && cache[endpoint]?.latency) {
        models.forEach((m) => {
            delete cache[endpoint].latency[m];
        });
        persist();
    }

    const pending = models.filter(
        (model) => (force || typeof cache[endpoint]?.latency?.[model] !== 'number') && !busy.has(model)
    );
    if (pending.length === 0) {
        notify();
        return;
    }
    pending.forEach((model) => busy.add(model));

    const queue = queues.get(endpoint) ?? { list: [], active: 0, total: 0, completed: 0 };
    queue.list.push(...pending);
    queue.total = (queue.total || 0) + pending.length;
    queue.apiKey = apiKey;
    queue.requestPath = requestPath;
    queue.fmt = formatOf(apiFormat);
    queues.set(endpoint, queue);
    notify();

    while (queue.active < CONCURRENCY && queue.list.length > 0) {
        queue.active += 1;
        void worker(endpoint, queue);
    }
}
