import i18next from 'i18next';

import * as umi from '../services/recognize/umi';
import * as wechat from '../services/recognize/wechat';
import { store } from './store';

// 这张表是「这个名字还有没有实现」的注册表，不是用户装了哪些 —— 后者存在
// store 的 recognize_service_list 里，顺序即优先级（识别只取第一个启用的返回，
// 不像翻译那样全跑一遍）。识别没有插件，也没有实例 key，一个服务一份配置。
export const RECOGNIZE_SERVICES = { wechat, umi };

// 微信 OCR 预装，Umi-OCR 要用户自己在服务设置里加。
export const DEFAULT_RECOGNIZE_LIST = ['wechat'];
export const RECOGNIZE_LIST_KEY = 'recognize_service_list';

// 配置存在 store 里，加 recognize_ 前缀是因为翻译那边已经占了 google 这个 key
// （谷歌翻译），不隔开会打架。
export const recognizeConfigKey = (name) => `recognize_${name}`;

export async function getRecognizeConfig(name) {
    return { ...RECOGNIZE_SERVICES[name].defaultConfig, ...((await store.get(recognizeConfigKey(name))) ?? {}) };
}

/// 用第一个启用的识别服务认一张图。一个能用的都没有就抛错。
export async function recognize(path) {
    const list = (await store.get(RECOGNIZE_LIST_KEY)) ?? DEFAULT_RECOGNIZE_LIST;
    for (const name of list) {
        // 列表里可能留着已经删掉的服务名（比如老版本的 google），跳过别炸。
        if (!RECOGNIZE_SERVICES[name]) continue;
        const config = await getRecognizeConfig(name);
        // 在列表里 = 已安装，默认就是开的，和翻译那边的 enable ?? true 一致。
        // 写成 !config.enable 的话，新装的服务因为 store 里还没这个字段会被静默跳过。
        if (config.enable === false) continue;
        return await RECOGNIZE_SERVICES[name].recognize(path, config);
    }
    throw new Error(i18next.t('config.service.no_recognize'));
}
