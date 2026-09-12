// Decode only the documented OCR fields. Transport success is not business success.
export function parseWechatResponse(value) {
    let json;
    try { json = JSON.parse(value); } catch { throw new Error('WeChat OCR: invalid JSON output'); }
    if (!json || !Number.isInteger(json.errcode)) throw new Error('WeChat OCR: invalid result status');
    if (json.errcode !== 0) throw new Error(`WeChat OCR: errcode=${json.errcode}`);
    if (!Array.isArray(json.ocr_response) || json.ocr_response.some((block) => !block || typeof block.text !== 'string')) {
        throw new Error('WeChat OCR: invalid text blocks');
    }
    return json.ocr_response
        .filter((block) => block.text.trim())
        .sort((a, b) => (Number.isFinite(a.top) ? a.top : 0) - (Number.isFinite(b.top) ? b.top : 0))
        .map((block) => block.text)
        .join('\n');
}
