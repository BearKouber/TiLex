// Umi-OCR 的响应判读。抽出来是为了能 node 直接跑自检 —— index.js 里带
// @tauri-apps 的 import，node 跑不起来。
//
//   code 100  认出字了，data 是块数组
//   code 101  图里没字，data 是一句说明文字
//   其他      出错了，data 是错误原文
export function parseUmiResponse(json) {
    if (json?.code === 100) {
        // Umi 已经按阅读顺序返回，不用像微信那份再按 top 排一次。
        return (json.data ?? [])
            .map((b) => b?.text ?? '')
            .filter((t) => t.trim())
            .join('\n');
    }
    if (json?.code === 101) {
        return '';
    }
    throw new Error(typeof json?.data === 'string' ? json.data : `Umi-OCR: code=${json?.code}`);
}
