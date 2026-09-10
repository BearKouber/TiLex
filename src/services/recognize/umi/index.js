import { readBinaryFile } from '@tauri-apps/api/fs';
import { fetch, Body } from '@tauri-apps/api/http';
import i18next from 'i18next';

import { parseUmiResponse } from './parse.js';

// Umi-OCR 本地离线识别。免费、不联网、不挑本机装了什么，代价是用户得自己开着
// Umi-OCR 并把 HTTP 接口打开（设置→全局设置→剪贴板/HTTP服务，默认端口 1224）。
//
//   POST http://127.0.0.1:1224/api/ocr
//   { "base64": "<不带 data: 前缀的 base64>" }
//
// 它替掉了原来的 Google Vision —— 那家要 GCP 收费 key，按量计费。

export const defaultConfig = {
    url: 'http://127.0.0.1:1224/api/ocr',
};

// 图片可能上兆，String.fromCharCode(...bytes) 那种写法会把参数铺到栈上直接爆掉，
// 所以分块喂。（从删掉的 google 那份搬过来的，坑一样。）
export function toBase64(bytes) {
    let binary = '';
    const CHUNK = 0x8000;
    for (let i = 0; i < bytes.length; i += CHUNK) {
        binary += String.fromCharCode.apply(null, bytes.subarray(i, i + CHUNK));
    }
    return btoa(binary);
}

/// 发一张 base64 图过去。测试连通性也走这条，所以单独抽出来。
export async function postBase64(url, base64) {
    let res;
    try {
        res = await fetch(url || defaultConfig.url, { method: 'POST', body: Body.json({ base64 }) });
    } catch {
        // 服务没启动时 tauri 直接 reject，底层文案是 connection refused，用户看不懂。
        throw new Error(i18next.t('config.service.umi_not_running'));
    }
    if (!res.ok) {
        throw new Error(`Umi-OCR: HTTP ${res.status}`);
    }
    return res.data;
}

export async function recognize(path, config = {}) {
    const base64 = toBase64(await readBinaryFile(path));
    return parseUmiResponse(await postBase64(config.url, base64));
}
