// 朗读走 WebView2 自带的 speechSynthesis：零依赖、离线、和用哪个翻译服务无关
// —— 拿到的是文本，谁翻的都一样能读。（TTS 服务那一整个目录在批次 3 删掉了。）

// 系统默认嗓音跟着系统语言走，用中文嗓音念英文会念成拼音。
// 有汉字就按中文读，否则按英文读；够用了，别再引语种识别。
const langOf = (text) => (/[\u4e00-\u9fff\u3040-\u30ff]/.test(text) ? 'zh-CN' : 'en-US');

export function speak(text) {
    if (!text) return;
    speechSynthesis.cancel();
    const utterance = new SpeechSynthesisUtterance(text);
    utterance.lang = langOf(text);
    speechSynthesis.speak(utterance);
}
