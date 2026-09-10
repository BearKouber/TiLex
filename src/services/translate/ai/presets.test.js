// Run: node src/services/translate/ai/presets.test.js
import assert from 'node:assert/strict';
import { AI_PRESETS, presetToConfig } from './presets.js';
import { matchIcon, ICONS, FALLBACK_ICON } from './icons.js';
import { FORMATS } from './protocol.js';

// 每个预设的地址经 protocol.js 补完之后必须落在真端点上。三个易错点：
// zhipu 的 /v4、gemini 的 /v1beta、qwen 的 /compatible-mode/v1 —— resolve()
// 的「已带版本号就不再补」规则如果改坏了，这三家会静默打歪。
// 预设不带模型名了，这里拿一个占位模型跑 chatUrl（只有 google 会把它嵌进路径）。
const M = 'probe-model';
const EXPECTED = {
    deepseek: 'https://api.deepseek.com/v1/chat/completions',
    claude: 'https://api.anthropic.com/v1/messages',
    gemini: `https://generativelanguage.googleapis.com/v1beta/models/${M}:generateContent`,
    openai: 'https://api.openai.com/v1/chat/completions',
    kimi: 'https://api.moonshot.cn/v1/chat/completions',
    zhipu: 'https://open.bigmodel.cn/api/paas/v4/chat/completions',
    qwen: 'https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions',
    siliconflow: 'https://api.siliconflow.cn/v1/chat/completions',
    mimo: 'https://api.xiaomimimo.com/v1/chat/completions',
};
assert.equal(AI_PRESETS.length, Object.keys(EXPECTED).length);
for (const p of AI_PRESETS) {
    assert.equal(FORMATS[p.apiFormat].chatUrl(p.requestPath, M), EXPECTED[p.id], p.id);
    // 图标 id 必须在表里，否则列表行会渲染成空白
    assert.ok(p.id in ICONS, `${p.id} 不在 ICONS 表里`);
    assert.equal(presetToConfig(p).icon, p.id);
    // 不许再带默认模型：写死的模型名会过期，也挡住新模型
    assert.equal(p.defaultModel, undefined, `${p.id} 不该带 defaultModel`);
    assert.equal(presetToConfig(p).model, undefined, `${p.id} 不该预填 model`);
}

// 硅基流动挂的是 DeepSeek 的模型，认脸必须按地址走
assert.equal(matchIcon('https://api.siliconflow.cn/v1', 'deepseek-ai/DeepSeek-V3.2-Exp'), 'siliconflow');
// 地址不认识时才轮到模型名
assert.equal(matchIcon('https://my-relay.example.com/v1', 'claude-sonnet-5'), 'claude');
assert.equal(matchIcon('http://127.0.0.1:11434/v1', ''), 'ollama');
assert.equal(matchIcon('https://open.bigmodel.cn/api/paas/v4', 'glm-4.6'), 'zhipu');
assert.equal(matchIcon('https://api.xiaomimimo.com/v1', ''), 'mimo');
// 都不认识：回落，绝不返回 undefined
assert.equal(matchIcon('', ''), FALLBACK_ICON);
assert.equal(matchIcon('https://unknown.example.com', 'my-model'), FALLBACK_ICON);

console.log('ai presets + icons: ok');
