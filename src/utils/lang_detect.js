import { fetch, Body } from '@tauri-apps/api/http';
import { invoke } from '@tauri-apps/api';
import { store } from './store';
import { v4 as uuidv4 } from 'uuid';

// https://fanyi-api.baidu.com/product/113
async function baidu_detect(text) {
    const lang_map = {
        zh: 'zh_cn',
        cht: 'zh_tw',
        en: 'en',
        jp: 'ja',
        kor: 'ko',
        fra: 'fr',
        spa: 'es',
        ru: 'ru',
        de: 'de',
        it: 'it',
        tr: 'tr',
        pt: 'pt_pt',
        vie: 'vi',
        id: 'id',
        th: 'th',
        may: 'ms',
        ar: 'ar',
        hi: 'hi',
        nob: 'nb_no',
        nno: 'nn_no',
        per: 'fa',
        ukr: 'uk'
    };
    let res = await fetch('https://fanyi.baidu.com/langdetect', {
        method: 'POST',
        headers: {
            'Content-Type': 'application/x-www-form-urlencoded',
        },
        body: Body.form({
            query: text,
        }),
    });
    if (res.ok) {
        let result = res.data;
        if (result.lan && result.lan in lang_map) {
            return lang_map[result.lan];
        }
    }
    return 'en';
}
// https://cloud.google.com/translate/docs/languages?hl=zh-cn
async function google_detect(text) {
    const lang_map = {
        'zh-CN': 'zh_cn',
        'zh-TW': 'zh_tw',
        ja: 'ja',
        en: 'en',
        ko: 'ko',
        fr: 'fr',
        es: 'es',
        ru: 'ru',
        de: 'de',
        it: 'it',
        tr: 'tr',
        pt: 'pt_pt',
        vi: 'vi',
        id: 'id',
        th: 'th',
        ms: 'ms',
        ar: 'ar',
        hi: 'hi',
        mn: 'mn_cy',
        km: 'km',
        fa: 'fa',
        no: 'nb_no',
        uk: 'uk'
    };
    let res = await fetch(
        `https://translate.google.com/translate_a/single?dt=at&dt=bd&dt=ex&dt=ld&dt=md&dt=qca&dt=rw&dt=rm&dt=ss&dt=t`,
        {
            method: 'GET',
            headers: { 'content-type': 'application/json' },
            query: {
                client: 'gtx',
                sl: 'auto',
                tl: 'zh-CN',
                hl: 'zh-CN',
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
        const result = res.data;
        if (result[2] && result[2] in lang_map) {
            return lang_map[result[2]];
        }
    }
    return 'en';
}
// https://niutrans.com/documents/contents/trans_text#languageList
async function niutrans_detect(text) {
    const lang_map = {
        zh: 'zh_cn',
        cht: 'zh_cn',
        en: 'en',
        ja: 'ja',
        ko: 'ko',
        fr: 'fr',
        es: 'es',
        ru: 'ru',
        de: 'de',
        it: 'it',
        tr: 'tr',
        pt: 'pt_pt',
        vi: 'vi',
        id: 'id',
        th: 'th',
        ms: 'ms',
        ar: 'ar',
        hi: 'hi',
        mn: 'mn_cy',
        mo: 'mn_mo',
        km: 'km',
        nb: 'nb_no',
        nn: 'nn_no',
        fa: 'fa',
        uk: 'uk'
    };
    let res = await fetch('https://test.niutrans.com/NiuTransServer/language', {
        method: 'GET',
        headers: { 'content-type': 'application/json' },
        query: {
            src_text: text,
            source: 'text',
            time: new String(new Date().getTime()),
        },
    });
    if (res.ok) {
        const result = res.data;
        if (result['language'] && result['language'] in lang_map) {
            return lang_map[result['language']];
        }
    }
    return 'en';
}
// https://yandex.com/dev/translate/doc/en/concepts/api-overview
async function yandex_detect(text) {
    const lang_map = {
        zh: 'zh_cn',
        en: 'en',
        ja: 'ja',
        ko: 'ko',
        fr: 'fr',
        es: 'es',
        ru: 'ru',
        de: 'de',
        it: 'it',
        tr: 'tr',
        pt: 'pt_pt',
        vi: 'vi',
        id: 'id',
        th: 'th',
        ms: 'ms',
        ar: 'ar',
        hi: 'hi',
        no: 'nb_no',
        fa: 'fa',
        uk: 'uk'
    };

    let res = await fetch('https://translate.yandex.net/api/v1/tr.json/detect', {
        method: 'GET',
        query: {
            id: uuidv4().replaceAll('-', '') + '-0-0',
            srv: 'android',
            text: text,
        },
    });
    if (res.ok) {
        const result = res.data;
        if (result['lang'] && result['lang'] in lang_map) {
            return lang_map[result['lang']];
        }
    }
    return 'en';
}
// 本地识别（lingua）：~0.2ms，离线。Rust 端用 --no-default-features
// 编译时会返回 Err，那时退到测下来最快的网络引擎。
async function local_detect(text) {
    try {
        return await invoke('lang_detect', { text: text });
    } catch {
        return await niutrans_detect(text);
    }
}

export default async function detect(text) {
    // 默认本地：实测 92-230µs，而网络引擎最快的 niutrans 也要 92ms、
    // google/baidu 要 1.2s。而且这个调用在 PopResult 里是阻塞翻译开始的。
    const engine = (await store.get('translate_detect_engine')) ?? 'local';

    switch (engine) {
        case 'baidu':
            return await baidu_detect(text);
        case 'google':
            return await google_detect(text);
        case 'niutrans':
            return await niutrans_detect(text);
        case 'yandex':
            return await yandex_detect(text);
        default:
            // 老配置里可能存着已删的 'tencent'，一并退到本地。
            return await local_detect(text);
    }
}
