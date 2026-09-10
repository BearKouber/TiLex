// tailwind.config.js
const { nextui } = require('@nextui-org/react');

/** @type {import('tailwindcss').Config} */
module.exports = {
    content: [
        // ...
        './index.html',
        './src/**/*.{js,ts,jsx,tsx}',
        './node_modules/@nextui-org/theme/dist/**/*.{js,ts,jsx,tsx}',
    ],
    theme: {
        extend: {},
    },
    darkMode: 'class',
    plugins: [
        nextui({
            // NextUI 的边框默认 medium=2px，太粗；控件要的是发丝边。
            // 圆角整体放大一档：所有控件（按钮 / 输入框 / 下拉 / 卡片）跟着圆润，
            // 不用逐个写 radius。
            layout: {
                borderWidth: { small: '1px', medium: '1px', large: '2px' },
                radius: { small: '8px', medium: '12px', large: '16px' },
            },
            themes: {
                // 黑白为主，靠亮度分层，单一蓝色强调。两套主题结构同构：
                // background 最暗/最浅，content1 是卡片且必须往「亮的方向」偏一档，
                // default-200 是发丝边框，default-500 是次要文字。
                // 这样组件的 class 都不用写 dark: 变体。
                light: {
                    colors: {
                        background: '#F7F8FA',
                        foreground: '#18181B',
                        content1: '#FFFFFF',
                        content2: '#FAFBFC',
                        content3: '#F1F2F4',
                        content4: '#E8E9EC',
                        default: {
                            // 控件是 variant='bordered'，这一档就是它的边框色，
                            // 要发丝级别，别往深里调。次要文字走 default-400/500，
                            // 不受这行影响。
                            DEFAULT: '#E4E6EA',
                            50: '#FAFBFC',
                            100: '#F4F5F7',
                            200: '#ECEDF0',
                            300: '#DEE0E4',
                            400: '#A1A1AA',
                            500: '#71717A',
                            600: '#52525B',
                            700: '#3F3F46',
                            800: '#27272A',
                            900: '#18181B',
                        },
                        primary: { DEFAULT: '#2563EB', foreground: '#FFFFFF' },
                        // 键盘 Tab 的焦点环。默认跟 primary 走，一圈蓝框和黑白风格打架，
                        // 改成浅灰；这是全局 token，所有控件一起变。
                        focus: '#D4D4D8',
                    },
                },
                dark: {
                    colors: {
                        background: '#0F0F10',
                        foreground: '#E4E4E7',
                        content1: '#18181B',
                        content2: '#1F1F23',
                        content3: '#27272A',
                        content4: '#2E2E33',
                        default: {
                            DEFAULT: '#52525B',
                            50: '#18181B',
                            100: '#1F1F23',
                            200: '#2A2A2E',
                            300: '#3F3F46',
                            400: '#71717A',
                            500: '#A1A1AA',
                            600: '#C4C4CA',
                            700: '#D4D4D8',
                            800: '#E4E4E7',
                            900: '#FAFAFA',
                        },
                        primary: { DEFAULT: '#3B82F6', foreground: '#FFFFFF' },
                        focus: '#71717A',
                    },
                },
            },
        }),
    ],
};
