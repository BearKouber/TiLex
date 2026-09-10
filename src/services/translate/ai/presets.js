// 知名 AI 厂商的开箱预设。选完只剩粘 API Key —— 地址和接口格式都是填好的。
//
// **刻意不带模型名。** 硬编码的模型一定会过期（第一版写的 claude-3-5-sonnet /
// gemini-1.5 / moonshot-v1 / glm-4 全都已经被取代了），而且指定死了反而挡住
// 新模型。模型交给配置弹窗里那个 ⚡ 按钮从 /v1/models 现拉，留空也能直接手打。
//
// requestPath 都要能被 protocol.js 的 resolve() 吃对（它「已带版本号就不再补」），
// zhipu 的 /v4、gemini 的 /v1beta、qwen 的 /compatible-mode/v1 是三个易错点，
// presets.test.js 拿断言盖住了。
//
// 没有 Ollama：设置里「第三方」那栏已经有它了，这里再放一个是两条路通同一处。

export const AI_PRESETS = [
    {
        id: 'deepseek',
        name: 'DeepSeek',
        requestPath: 'https://api.deepseek.com',
        apiFormat: 'openai_chat',
        needApiKey: true,
        helpUrl: 'https://platform.deepseek.com',
    },
    {
        id: 'claude',
        name: 'Anthropic Claude',
        requestPath: 'https://api.anthropic.com',
        apiFormat: 'anthropic',
        needApiKey: true,
        helpUrl: 'https://console.anthropic.com',
    },
    {
        id: 'gemini',
        name: 'Google Gemini',
        requestPath: 'https://generativelanguage.googleapis.com',
        apiFormat: 'google',
        needApiKey: true,
        helpUrl: 'https://aistudio.google.com',
    },
    {
        id: 'openai',
        name: 'OpenAI',
        requestPath: 'https://api.openai.com/v1',
        apiFormat: 'openai_chat',
        needApiKey: true,
        helpUrl: 'https://platform.openai.com',
    },
    {
        id: 'kimi',
        name: 'Moonshot Kimi',
        requestPath: 'https://api.moonshot.cn/v1',
        apiFormat: 'openai_chat',
        needApiKey: true,
        helpUrl: 'https://platform.moonshot.cn',
    },
    {
        id: 'zhipu',
        name: '智谱 GLM',
        requestPath: 'https://open.bigmodel.cn/api/paas/v4',
        apiFormat: 'openai_chat',
        needApiKey: true,
        helpUrl: 'https://open.bigmodel.cn',
    },
    {
        id: 'qwen',
        name: '通义千问',
        requestPath: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
        apiFormat: 'openai_chat',
        needApiKey: true,
        helpUrl: 'https://bailian.console.aliyun.com',
    },
    {
        id: 'siliconflow',
        name: 'SiliconFlow',
        requestPath: 'https://api.siliconflow.cn/v1',
        apiFormat: 'openai_chat',
        needApiKey: true,
        helpUrl: 'https://cloud.siliconflow.cn',
    },
    {
        id: 'mimo',
        name: '小米 MiMo',
        requestPath: 'https://api.xiaomimimo.com/v1',
        apiFormat: 'openai_chat',
        needApiKey: true,
        helpUrl: 'https://mimo.mi.com',
    },
];

export const presetOf = (id) => AI_PRESETS.find((p) => p.id === id) ?? null;

// 预设 → 配置弹窗默认值里要盖掉的那几项。model 不在里面，走 Config.jsx 的空
// 默认值；剩下的（promptList / requestArguments / stream）同理。
export function presetToConfig(preset) {
    return {
        instanceName: preset.name,
        requestPath: preset.requestPath,
        apiFormat: preset.apiFormat,
        apiKey: '',
        icon: preset.id,
        // 预设选定的图标就是对的，别再被地址匹配改掉
        iconLocked: true,
    };
}
