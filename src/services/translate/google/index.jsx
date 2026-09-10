import { fetch, Body } from '@tauri-apps/api/http';

export async function translate(text, from, to, options = {}) {
    const { config = {} } = options;
    const mode = config.type || 'web';

    if (mode === 'api') {
        return translate_by_api(text, from, to, config.api_key, config.custom_url);
    } else if (mode === 'custom_api') {
        return translate_by_custom_api(text, from, to, config.custom_api_url);
    } else {
        return translate_by_web(text, from, to, config.custom_url);
    }
}

function decodeHtmlEntities(str) {
    if (!str) return '';
    return str
        .replace(/&amp;/g, '&')
        .replace(/&lt;/g, '<')
        .replace(/&gt;/g, '>')
        .replace(/&quot;/g, '"')
        .replace(/&#39;/g, "'")
        .replace(/&#x27;/g, "'");
}

async function translate_by_web(text, from, to, custom_url) {
    let url = custom_url;
    if (url === undefined || url === '') {
        url = 'https://translate.google.com';
    }
    if (!url.startsWith('http')) {
        url = 'https://' + url;
    }
    url = url.replace(/\/+$/, '');

    let res = await fetch(
        `${url}/translate_a/single?dt=at&dt=bd&dt=ex&dt=ld&dt=md&dt=qca&dt=rw&dt=rm&dt=ss&dt=t`,
        {
            method: 'GET',
            headers: { 'content-type': 'application/json' },
            query: {
                client: 'gtx',
                sl: from,
                tl: to,
                hl: to,
                ie: 'UTF-8',
                oe: 'UTF-8',
                otf: '1',
                ssel: '0',
                tsel: '0',
                kc: '7',
                q: text,
            },
        }
    );
    if (res.ok) {
        let result = res.data;
        // 词典模式
        if (result[1]) {
            let target = { pronunciations: [], explanations: [], associations: [], sentence: [] };
            // 发音
            if (result[0]?.[1]?.[3]) {
                target.pronunciations.push({ symbol: result[0][1][3], voice: '' });
            }
            // 释义
            for (let i of result[1]) {
                target.explanations.push({
                    trait: i[0],
                    explains: i[2].map((x) => x[0]),
                });
            }
            // 例句
            if (result[13]) {
                for (let i of result[13][0]) {
                    target.sentence.push({ source: i[0] });
                }
            }
            return target;
        } else {
            // 翻译模式
            let target = '';
            for (let r of result[0]) {
                if (r[0]) {
                    target = target + r[0];
                }
            }
            return target.trim();
        }
    } else {
        throw new Error(`Http Request Error (Status: ${res.status})\n${JSON.stringify(res.data)}`);
    }
}

async function translate_by_api(text, from, to, apiKey, endpoint = 'https://translation.googleapis.com') {
    if (!apiKey || apiKey.trim() === '') {
        throw new Error('Please configure Google Cloud API Key');
    }
    let baseUrl = (endpoint || 'https://translation.googleapis.com').trim().replace(/\/+$/, '');
    if (!baseUrl.startsWith('http')) {
        baseUrl = 'https://' + baseUrl;
    }
    const url = `${baseUrl}/language/translate/v2`;
    const res = await fetch(url, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        query: { key: apiKey.trim() },
        body: Body.json({
            q: [text],
            target: to,
            source: from === 'auto' ? undefined : from,
            format: 'text',
        }),
    });
    if (res.ok) {
        const data = res.data;
        const translation = data?.data?.translations?.[0]?.translatedText;
        if (translation) {
            return decodeHtmlEntities(translation).trim();
        }
        throw new Error(`No translation returned:\n${JSON.stringify(data)}`);
    } else {
        throw new Error(`Google Cloud API Error (Status: ${res.status})\n${JSON.stringify(res.data)}`);
    }
}

async function translate_by_custom_api(text, from, to, url) {
    if (!url || url.trim() === '') {
        throw new Error('Please configure Custom API URL');
    }
    let targetUrl = url.trim();
    if (!targetUrl.startsWith('http')) {
        targetUrl = 'https://' + targetUrl;
    }
    const res = await fetch(targetUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: Body.json({
            text,
            source_lang: from,
            target_lang: to,
        }),
    });
    if (res.ok) {
        const data = res.data;
        if (typeof data === 'string') return data.trim();
        if (data?.data) return (typeof data.data === 'string' ? data.data : JSON.stringify(data.data)).trim();
        if (data?.translation) return data.translation.trim();
        if (data?.result) return (typeof data.result === 'string' ? data.result : JSON.stringify(data.result)).trim();
        throw new Error(`Unexpected API response structure:\n${JSON.stringify(data)}`);
    } else {
        throw new Error(`Http Request Error (Status: ${res.status})\n${JSON.stringify(res.data)}`);
    }
}

export * from './Config';
export * from './info';
