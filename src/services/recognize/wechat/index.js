import { invoke } from '@tauri-apps/api/tauri';
import { parseWechatResponse } from './parse.js';

// 微信 OCR。识别能力全部来自用户本机装的微信，路径两条都是自动探测的
// （见 src-tauri/src/ocr.rs），所以没有 key、没有地址、没有模型可配。
//
// 离线、不花钱、中文识别比 Windows.Media.Ocr 准不少，代价是必须装微信并登录过一次。

export const defaultConfig = {};

// wcocr 原样吐回来的 JSON 里只有 ocr_response[].text 有用。details 是逐字坐标，
// 我们只要整段文字，用不上。按 top 排序再换行拼 —— 它给的顺序不保证是从上到下。
export async function recognize(path) {
    try {
        return parseWechatResponse(await invoke('ocr_image', { path }));
    } catch (error) {
        // Preserve the shared, localized no-text handling in runOcrRequest.
        if (error === 'OCR_NO_TEXT') return '';
        throw error;
    }
}
