// Run: node src/services/translate/ai/protocol.test.js
import assert from 'node:assert/strict';
import { FORMATS, formatOf, DEFAULT_FORMAT } from './protocol.js';

const chatUrl = (p) => FORMATS.openai_chat.chatUrl(p);
const modelsUrl = (p) => FORMATS.openai_chat.modelsUrl(p);

// 完整的 completions 地址：models 换掉最后一段，chat 原样
assert.equal(modelsUrl('https://api.deepseek.com/v1/chat/completions'), 'https://api.deepseek.com/v1/models');
assert.equal(chatUrl('https://api.deepseek.com/v1/chat/completions'), 'https://api.deepseek.com/v1/chat/completions');

// 只给 base：两个地址都补 /v1/...
assert.equal(modelsUrl('https://api.openai.com'), 'https://api.openai.com/v1/models');
assert.equal(chatUrl('https://api.openai.com'), 'https://api.openai.com/v1/chat/completions');
assert.equal(modelsUrl('https://api.openai.com/'), 'https://api.openai.com/v1/models');

// 挂在子路径上的端点 —— 用 origin 拼就会在这里推错
assert.equal(modelsUrl('https://host/api/v1/chat/completions'), 'https://host/api/v1/models');
assert.equal(modelsUrl('https://host/api'), 'https://host/api/v1/models');
assert.equal(chatUrl('https://host/api'), 'https://host/api/v1/chat/completions');

// 没写协议头
assert.equal(modelsUrl('api.openai.com'), 'https://api.openai.com/v1/models');
assert.equal(chatUrl('api.openai.com'), 'https://api.openai.com/v1/chat/completions');

// 只给到 /v1 —— 各家文档现在都是这个写法，不能再补一层 v1
assert.equal(chatUrl('https://api.deepseek.com/v1'), 'https://api.deepseek.com/v1/chat/completions');
assert.equal(modelsUrl('https://api.deepseek.com/v1'), 'https://api.deepseek.com/v1/models');
assert.equal(chatUrl('https://api.deepseek.com/v1/'), 'https://api.deepseek.com/v1/chat/completions');
assert.equal(chatUrl('https://host/api/v2'), 'https://host/api/v2/chat/completions');

// 本地端口
assert.equal(chatUrl('http://127.0.0.1:8045/'), 'http://127.0.0.1:8045/v1/chat/completions');
assert.equal(chatUrl('http://127.0.0.1:8045'), 'http://127.0.0.1:8045/v1/chat/completions');
assert.equal(modelsUrl('http://127.0.0.1:8045/'), 'http://127.0.0.1:8045/v1/models');

// —— 四种接口格式 ——

// 地址：Google 把模型名塞在路径里，另外三家不
assert.equal(FORMATS.openai_responses.chatUrl('https://api.openai.com'), 'https://api.openai.com/v1/responses');
assert.equal(FORMATS.anthropic.chatUrl('https://api.anthropic.com'), 'https://api.anthropic.com/v1/messages');
assert.equal(
    FORMATS.google.chatUrl('https://generativelanguage.googleapis.com', 'gemini-2.0-flash'),
    'https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent'
);
// 用户已经填了版本号就不再补一层（v1beta 也算版本号）
assert.equal(FORMATS.google.modelsUrl('https://host/x/v1beta'), 'https://host/x/v1beta/models');
// 已经填了别家的端点段，换格式时要能剥掉重拼
assert.equal(FORMATS.anthropic.chatUrl('https://host/v1/chat/completions'), 'https://host/v1/messages');

// 认证头
assert.deepEqual(FORMATS.openai_chat.headers('k'), { Authorization: 'Bearer k' });
assert.deepEqual(FORMATS.anthropic.headers('k'), { 'x-api-key': 'k', 'anthropic-version': '2023-06-01' });
assert.deepEqual(FORMATS.google.headers('k'), { 'x-goog-api-key': 'k' });

const PROMPT = [
    { role: 'system', content: 'sys' },
    { role: 'user', content: 'hi' },
];
const ARGS = { temperature: 0.1, top_p: 0.99, frequency_penalty: 0, presence_penalty: 0 };

// OpenAI 两家：system 留在消息列表里，参数原样透传
assert.deepEqual(FORMATS.openai_chat.body('m', PROMPT, ARGS).messages, PROMPT);
assert.equal(FORMATS.openai_chat.body('m', PROMPT, ARGS).frequency_penalty, 0);
assert.deepEqual(FORMATS.openai_responses.body('m', PROMPT, ARGS).input, PROMPT);

// Anthropic：system 单独一个字段，max_tokens 必填，penalty 不能传（传了 400）
const ab = FORMATS.anthropic.body('m', PROMPT, ARGS);
assert.equal(ab.system, 'sys');
assert.deepEqual(ab.messages, [{ role: 'user', content: 'hi' }]);
assert.equal(ab.max_tokens, 4096);
assert.equal(FORMATS.anthropic.body('m', PROMPT, { max_tokens: 5 }).max_tokens, 5);
assert.equal('frequency_penalty' in ab, false);
assert.equal('presence_penalty' in ab, false);

// Google：systemInstruction + contents，参数搬进 generationConfig 并改名
const gb = FORMATS.google.body('m', PROMPT, ARGS);
assert.deepEqual(gb.systemInstruction, { parts: [{ text: 'sys' }] });
assert.deepEqual(gb.contents, [{ role: 'user', parts: [{ text: 'hi' }] }]);
assert.equal(gb.generationConfig.topP, 0.99);
assert.equal('top_p' in gb.generationConfig, false);
assert.equal('frequency_penalty' in gb.generationConfig, false);

// 响应取文本
assert.equal(FORMATS.openai_chat.text({ choices: [{ message: { content: 'a' } }] }), 'a');
assert.equal(FORMATS.openai_responses.text({ output: [{ content: [{ text: 'a' }, { text: 'b' }] }] }), 'ab');
assert.equal(FORMATS.anthropic.text({ content: [{ text: 'a' }] }), 'a');
assert.equal(FORMATS.google.text({ candidates: [{ content: { parts: [{ text: 'a' }] } }] }), 'a');
// 取不到就是 undefined，调用方按「空结果」处理，不许抛
assert.equal(FORMATS.openai_chat.text({ error: 'x' }), undefined);

// 模型列表
assert.deepEqual(FORMATS.openai_chat.models({ data: [{ id: 'a' }] }), ['a']);
assert.deepEqual(FORMATS.google.models({ models: [{ name: 'models/gemini-x' }] }), ['gemini-x']);
assert.equal(FORMATS.openai_chat.models({}), null);

// 旧配置没有 apiFormat 字段，必须回落到原来那套
assert.equal(formatOf(undefined), FORMATS[DEFAULT_FORMAT]);
assert.equal(formatOf('nonsense'), FORMATS.openai_chat);

console.log('ok');
