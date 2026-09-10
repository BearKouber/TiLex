import * as _bing from './bing';
import * as _google from './google';
import * as _transmart from './transmart';
import * as _baidu from './baidu';
import * as _deepl from './deepl';
import * as _ai from './ai';

// 免费开箱即用（支持内置免配与 API 模式）
export const bing = _bing;
export const google = _google;
export const transmart = _transmart;

// 需配置凭证或自建服务
export const baidu = _baidu;
export const deepl = _deepl;

// ai 是自定义 AI 服务的载体，所有 AI 实例都是它的实例（方案 §4）
export const ai = _ai;

