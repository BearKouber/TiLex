import { fetch, Body } from '@tauri-apps/api/http';
import { isWord } from '../../../utils/wordbook_format';
import { dictionaryResult } from './dictionary';
import { Language } from './info';
import { defaultRequestArguments } from './Config';
import { formatOf } from './protocol';

// 划到单个词时，直译一个词还不如谷歌 —— 谷歌给的是词典条目（音标、词性、
// 多条释义）。所以词走一套词典 prompt，要模型直接吐 google 那个形状的 JSON：
// 悬浮窗的 DictView 和收藏落库都认这个形状，两边一行都不用改。
const DICT_PROMPT = `你是一部双语词典。为给出的词条输出词典条目，释义写成 $to，只输出合法 JSON，不要围栏、不要解释：
{
  "pronunciations": [{ "symbol": "/音标/" }],
  "explanations": [{ "trait": "词性缩写，如 n. / v. / adj.", "explains": ["释义1", "释义2"] }],
  "associations": ["常见搭配或变形，最多 3 条"]
}
每个词性一条 explanations；没有音标就给空数组，associations 也可以为空数组。`;

export async function translate(text, from, to, options) {
    const { config, detect } = options;

    let { requestPath, model, apiKey, promptList, requestArguments, apiFormat } = config;

    // 地址补全 / 认证头 / 请求体 / 响应解析全在 protocol.js 那张表里，
    // 测速和翻译走同一条 —— 以前这里自己抄了一遍补全规则，
    // 填 base 到 /v1 的时候会推成 /v1/v1/chat/completions。
    const fmt = formatOf(apiFormat);
    const apiUrl = fmt.chatUrl(requestPath, model);

    // 词典模式：词条走 DICT_PROMPT，长句仍然走用户配置的翻译 prompt。
    // 判定规则和生词本入库分流是同一条（≤2 个词、末尾不是句末标点）。
    const dict = isWord(text);
    if (dict) {
        promptList = [
            { role: 'system', content: DICT_PROMPT },
            { role: 'user', content: '$text' },
        ];
    }

    // 兼容旧版
    if (promptList === undefined) {
        promptList = [
            {
                role: 'system',
                content:
                    'You are a professional translation engine, please translate the text into a colloquial, professional, elegant and fluent content, without the style of machine translation. You must only translate the text content, never interpret it.',
            },
            { role: 'user', content: `Translate into $to:\n"""\n$text\n"""` },
        ];
    }

    promptList = promptList.map((item) => {
        return {
            ...item,
            content: item.content
                .replaceAll('$text', text)
                .replaceAll('$from', from)
                .replaceAll('$to', to)
                .replaceAll('$detect', Language[detect]),
        };
    });

    // 流式那条路和它的开关一起删了，理由见 Config.jsx 的默认配置
    const args = JSON.parse(requestArguments ?? defaultRequestArguments);
    let res = await fetch(apiUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...fmt.headers(apiKey) },
        body: Body.json(fmt.body(model, promptList, args)),
    });
    if (res.ok) {
        let target = fmt.text(res.data)?.trim();
        if (target) {
            if (dict) {
                // 解析不出来就退回纯文本，最差也就是和改之前一样。
                return dictionaryResult(target);
            }
            if (target.startsWith('"')) {
                target = target.slice(1);
            }
            if (target.endsWith('"')) {
                target = target.slice(0, -1);
            }
            return target.trim();
        } else {
            throw JSON.stringify(res.data);
        }
    } else {
        throw `Http Request Error\nHttp Status: ${res.status}\n${JSON.stringify(res.data)}`;
    }
}

export * from './Config';
export * from './info';
