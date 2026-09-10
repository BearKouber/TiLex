import { fetch, Body } from '@tauri-apps/api/http';

export async function translate(text, from, to) {
    const url = 'https://transmart.qq.com/api/imt';
    const res = await fetch(url, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
            Referer: 'https://yi.qq.com/zh-CN/index',
            'User-Agent':
                'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/110.0.0.0 Safari/537.36',
        },
        body: Body.json({
            header: {
                fn: 'auto_translation_block',
                client_key:
                    'browser-chrome-110.0.0-Mac OS-df4bd4c5-a65d-44b2-a40f-42f34f3535f2-1677486696487',
            },
            type: 'plain',
            model_category: 'normal',
            source: {
                lang: from || 'auto',
                text_block: text,
            },
            target: {
                lang: to || 'zh',
            },
        }),
    });

    if (res.ok) {
        const data = res.data;
        if (data?.auto_translation) {
            return data.auto_translation.trim();
        }
        throw new Error(`Invalid response: ${JSON.stringify(data)}`);
    } else {
        throw new Error(`Http Status: ${res.status}\n${JSON.stringify(res.data)}`);
    }
}

export * from './Config';
export * from './info';
