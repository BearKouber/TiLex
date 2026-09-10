// 一个「接口格式」= 地址怎么补 + 认证头怎么写 + 请求体什么形状 + 响应怎么取文本。
// 四家的差别就这四条，全部收在这张表里；index.jsx / latency.js / Config.jsx
// 都只认这张表，加第五家只用往下面再写一条。

// requestPath 是用户手填的，形态很自由：可能是完整端点，可能只到 /v1，
// 可能只是个 base，可能没有协议头，也可能挂在子路径上。
const KNOWN_TAILS = /\/(chat\/completions|responses|messages|models)$/;

function resolve(requestPath, tail, version) {
    if (!/https?:\/\/.+/.test(requestPath)) {
        requestPath = `https://${requestPath}`;
    }
    const url = new URL(requestPath);
    // 末尾斜杠和已经填上的端点段先去掉，后面统一自己拼。
    // 注意不能写回 url.pathname —— 赋空串它会自己变回 '/'，就多出一道斜杠。
    let path = url.pathname.replace(/\/$/, '').replace(KNOWN_TAILS, '');
    // 已经带版本号（/v1、/v2、聚合站的 /v3、Google 的 /v1beta）就不再补
    if (!/\/v\d+[a-z]*$/.test(path)) {
        path = `${path}/${version}`;
    }
    url.pathname = `${path}/${tail}`;
    return url.href;
}

const num = (v) => (typeof v === 'number' ? v : undefined);

export const FORMATS = {
    openai_chat: {
        label: 'OpenAI Chat Completions',
        chatUrl: (base) => resolve(base, 'chat/completions', 'v1'),
        modelsUrl: (base) => resolve(base, 'models', 'v1'),
        headers: (apiKey) => ({ Authorization: `Bearer ${apiKey}` }),
        body: (model, messages, args) => ({ ...args, stream: false, model, messages }),
        text: (d) => d?.choices?.[0]?.message?.content,
        models: (d) => (Array.isArray(d?.data) ? d.data.map((m) => m.id) : null),
    },
    openai_responses: {
        label: 'OpenAI Responses',
        chatUrl: (base) => resolve(base, 'responses', 'v1'),
        modelsUrl: (base) => resolve(base, 'models', 'v1'),
        headers: (apiKey) => ({ Authorization: `Bearer ${apiKey}` }),
        // Responses 用 input 而不是 messages，形状照样是 {role, content}
        body: (model, messages, args) => ({ ...args, stream: false, model, input: messages }),
        text: (d) =>
            d?.output_text ??
            d?.output
                ?.flatMap((o) => o.content ?? [])
                .map((c) => c.text)
                .filter(Boolean)
                .join(''),
        models: (d) => (Array.isArray(d?.data) ? d.data.map((m) => m.id) : null),
    },
    anthropic: {
        label: 'Anthropic Messages',
        chatUrl: (base) => resolve(base, 'messages', 'v1'),
        modelsUrl: (base) => resolve(base, 'models', 'v1'),
        headers: (apiKey) => ({ 'x-api-key': apiKey, 'anthropic-version': '2023-06-01' }),
        body: (model, messages, args) => ({
            model,
            // Anthropic 必填。用户的 requestArguments 里没有就给个够用的默认值。
            max_tokens: num(args.max_tokens) ?? 4096,
            // system 不在 messages 里，要单独一个字段
            ...(() => {
                const system = messages
                    .filter((m) => m.role === 'system')
                    .map((m) => m.content)
                    .join('\n\n');
                return system ? { system } : {};
            })(),
            messages: messages.filter((m) => m.role !== 'system'),
            // frequency_penalty / presence_penalty 传过去直接 400，只挑认识的
            ...(num(args.temperature) !== undefined ? { temperature: args.temperature } : {}),
            ...(num(args.top_p) !== undefined ? { top_p: args.top_p } : {}),
        }),
        text: (d) =>
            d?.content
                ?.map((c) => c.text)
                .filter(Boolean)
                .join(''),
        models: (d) => (Array.isArray(d?.data) ? d.data.map((m) => m.id) : null),
    },
    google: {
        label: 'Google Generative AI',
        // 只有这家把模型名塞在地址里
        chatUrl: (base, model) => resolve(base, `models/${model}:generateContent`, 'v1beta'),
        modelsUrl: (base) => resolve(base, 'models', 'v1beta'),
        headers: (apiKey) => ({ 'x-goog-api-key': apiKey }),
        body: (model, messages, args) => ({
            ...(() => {
                const system = messages
                    .filter((m) => m.role === 'system')
                    .map((m) => m.content)
                    .join('\n\n');
                return system ? { systemInstruction: { parts: [{ text: system }] } } : {};
            })(),
            contents: messages
                .filter((m) => m.role !== 'system')
                .map((m) => ({ role: m.role === 'assistant' ? 'model' : 'user', parts: [{ text: m.content }] })),
            generationConfig: {
                ...(num(args.temperature) !== undefined ? { temperature: args.temperature } : {}),
                ...(num(args.top_p) !== undefined ? { topP: args.top_p } : {}),
                ...(num(args.max_tokens) !== undefined ? { maxOutputTokens: args.max_tokens } : {}),
            },
        }),
        text: (d) =>
            d?.candidates?.[0]?.content?.parts
                ?.map((p) => p.text)
                .filter(Boolean)
                .join(''),
        models: (d) =>
            Array.isArray(d?.models) ? d.models.map((m) => (m.name ?? '').replace(/^models\//, '')) : null,
    },
};

export const DEFAULT_FORMAT = 'openai_chat';
// 旧配置没有 apiFormat 这个字段，一律当成原来那套 OpenAI Chat Completions
export const formatOf = (key) => FORMATS[key] ?? FORMATS[DEFAULT_FORMAT];
