import { initReactI18next } from 'react-i18next';
import i18n from 'i18next';
import zh_CN from './locales/zh_CN.json';
import en_US from './locales/en_US.json';

// http://www.lingoes.net/zh/translator/langcode.htm

i18n.use(initReactI18next).init({
    // 只保留中英两份文案，其余语言全部回落到英文
    fallbackLng: ['en'],
    debug: false,
    interpolation: {
        escapeValue: false,
    },
    resources: {
        en: en_US,
        zh_cn: zh_CN,
    },
});

export default i18n;
