// AI 实例的「认脸」层：一张 id → 外观的表，加一个从地址/模型名猜厂商的函数。
// 纯数据，不含 JSX —— 渲染在 ServiceIcon.jsx，这样 presets.test.js 能拿 node 直接跑。
//
// 图标资产分三档：public/logo/ 下有真 SVG 的（OpenAI / DeepSeek 那条鲸鱼，取自
// simple-icons，CC0）走 file；react-icons 5.3 里有的走组件；剩下的（Ollama /
// Kimi / 智谱 / SiliconFlow / Grok）用品牌色 + 首字的方块顶着 —— 商标不能凭
// 记忆画。以后拿到真图标：丢进 public/logo/，在这张表里给那行补 file 即可。
// （不做「有没有这个文件」的自动探测 —— 缺文件时 tauri 的资源协议不一定返回
// 404，<img onError> 可能永远不触发，那就是一个不会回落的破图。）
export const ICONS = {
    openai: { label: 'OpenAI', color: '#10A37F', letter: 'O', file: 'logo/openai.svg' },
    claude: { label: 'Claude', color: '#D97757', letter: 'C' },
    gemini: { label: 'Gemini', color: '#4285F4', letter: 'G' },
    qwen: { label: '通义千问', color: '#FF6A00', letter: '通' },
    copilot: { label: 'Copilot', color: '#6E56CF', letter: 'C' },
    deepseek: { label: 'DeepSeek', color: '#4D6BFE', letter: 'D', file: 'logo/deepseek.svg' },
    ollama: { label: 'Ollama', color: '#334155', letter: 'O' },
    kimi: { label: 'Kimi', color: '#16191E', letter: 'K' },
    zhipu: { label: '智谱 GLM', color: '#3859FF', letter: '智' },
    siliconflow: { label: 'SiliconFlow', color: '#6E56CF', letter: 'S' },
    mimo: { label: '小米 MiMo', color: '#FF6900', letter: 'M' },
    grok: { label: 'Grok', color: '#1F2937', letter: 'X' },
    sparkle: { label: 'AI', color: '#8B5CF6', letter: '✦' },
};

export const FALLBACK_ICON = 'sparkle';
export const ICON_IDS = Object.keys(ICONS);

// 关键词 → 图标 id。顺序有讲究：siliconflow 的默认模型是
// deepseek-ai/DeepSeek-V3.2-Exp，所以它必须排在 deepseek 前面，否则硅基流动
// 永远顶着 DeepSeek 的脸。
const RULES = [
    [/siliconflow|siliconcloud/, 'siliconflow'],
    [/deepseek/, 'deepseek'],
    [/anthropic|claude/, 'claude'],
    [/gemini|generativelanguage/, 'gemini'],
    [/ollama|11434/, 'ollama'],
    [/moonshot|kimi/, 'kimi'],
    [/bigmodel|zhipu|glm/, 'zhipu'],
    [/dashscope|qwen|aliyun|tongyi/, 'qwen'],
    [/xiaomimimo|mimo/, 'mimo'],
    [/grok|x\.ai/, 'grok'],
    [/copilot/, 'copilot'],
    [/openai|chatgpt|gpt-/, 'openai'],
];

/// 从请求地址和模型名猜是哪家。地址比模型名可信（聚合站会挂别家的模型），
/// 所以先单独拿地址跑一遍，没中再连模型名一起跑。
export function matchIcon(requestPath = '', model = '') {
    const path = String(requestPath).toLowerCase();
    const both = `${path} ${String(model).toLowerCase()}`;
    for (const source of [path, both]) {
        if (!source.trim()) continue;
        for (const [re, id] of RULES) {
            if (re.test(source)) return id;
        }
    }
    return FALLBACK_ICON;
}
