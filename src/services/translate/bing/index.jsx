import { fetch, Body } from '@tauri-apps/api/http';
import CryptoJS from 'crypto-js';
import { v4 as uuidv4 } from 'uuid';

const PRIVATE_KEY_HEX =
    'a2293a3dd0dd3273977a64dbc2f327f5d7bf87d9459df05a0966c630c66aaa849a41aa943aa8d51a6e4daac9a3701235c7eb12f6e823079e471095918855d817';
const PRIVATE_KEY = CryptoJS.enc.Hex.parse(PRIVATE_KEY_HEX);

function getSignature(url) {
    const guid = uuidv4().replace(/-/g, '');
    const escapedUrl = encodeURIComponent(url);
    const dateTime = new Date().toUTCString();
    const signStr = `MSTranslatorAndroidApp${escapedUrl}${dateTime}${guid}`.toLowerCase();
    const hash = CryptoJS.HmacSHA256(signStr, PRIVATE_KEY).toString(CryptoJS.enc.Base64);
    return `MSTranslatorAndroidApp::${hash}::${dateTime}::${guid}`;
}

export async function translate(text, from, to, options = {}) {
    const { config = {} } = options;
    const mode = config.type || 'builtin';

    if (mode === 'api') {
        return translate_by_api(text, from, to, config);
    } else {
        return translate_by_builtin(text, from, to);
    }
}

async function translate_by_builtin(text, from, to) {
    let requestPath = `api.cognitive.microsofttranslator.com/translate?api-version=3.0&to=${to}`;
    if (from && from !== 'auto' && from !== '') {
        requestPath += `&from=${from}`;
    }

    const sig = getSignature(requestPath);

    const res = await fetch(`https://${requestPath}`, {
        method: 'POST',
        headers: {
            'Content-Type': 'application/json',
            'User-Agent':
                'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/127.0.0.0 Safari/537.36',
            'X-MT-Signature': sig,
        },
        body: Body.json([{ Text: text }]),
    });

    if (res.ok) {
        const result = res.data;
        if (result?.[0]?.translations?.[0]?.text) {
            return result[0].translations[0].text.trim();
        }
        throw new Error(JSON.stringify(result));
    } else {
        throw new Error(`Http Request Error (Status: ${res.status})\n${JSON.stringify(res.data)}`);
    }
}

async function translate_by_api(text, from, to, config) {
    const authKey = config.auth_key?.trim();
    if (!authKey) {
        throw new Error('Please configure Azure Translator Auth Key');
    }
    let endpoint = (config.custom_url?.trim() || 'https://api.cognitive.microsofttranslator.com').replace(/\/+$/, '');
    if (!endpoint.startsWith('http')) {
        endpoint = 'https://' + endpoint;
    }

    const headers = {
        'Content-Type': 'application/json',
        'Ocp-Apim-Subscription-Key': authKey,
    };
    if (config.region && config.region.trim() !== '') {
        headers['Ocp-Apim-Subscription-Region'] = config.region.trim();
    }

    const query = {
        'api-version': '3.0',
        to: to,
    };
    if (from && from !== 'auto' && from !== '') {
        query.from = from;
    }

    const res = await fetch(`${endpoint}/translate`, {
        method: 'POST',
        headers,
        query,
        body: Body.json([{ Text: text }]),
    });

    if (res.ok) {
        const result = res.data;
        if (result?.[0]?.translations?.[0]?.text) {
            return result[0].translations[0].text.trim();
        }
        throw new Error(JSON.stringify(result));
    } else {
        throw new Error(`Azure API Request Error (Status: ${res.status})\n${JSON.stringify(res.data)}`);
    }
}

export * from './Config';
export * from './info';
