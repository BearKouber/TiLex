import { MdLightMode, MdDarkMode, MdSettingsBrightness } from 'react-icons/md';
import { useNavigate, useLocation, useRoutes } from 'react-router-dom';
import { Tabs, Tab, Button, Tooltip } from '@nextui-org/react';
import { appWindow } from '@tauri-apps/api/window';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api';
import { useTheme } from 'next-themes';
import { Toaster } from 'react-hot-toast';
import React, { useEffect } from 'react';

import WindowControl from '../../components/WindowControl';
import { osType } from '../../utils/env';
import { useConfig } from '../../hooks';
import routes from './routes';
import './style.css';

// 亮 → 暗 → 跟随系统，循环。图标就是当前值。
const THEME_CYCLE = { light: 'dark', dark: 'system', system: 'light' };
const THEME_ICON = {
    light: <MdLightMode className='text-[18px]' />,
    dark: <MdDarkMode className='text-[18px]' />,
    system: <MdSettingsBrightness className='text-[18px]' />,
};

const TABS = ['translate', 'service', 'wordbook', 'about'];

export default function Config() {
    const [appLanguage, setAppLanguage] = useConfig('app_language', 'en');
    const [appTheme, setAppTheme] = useConfig('app_theme', 'system');
    const { t, i18n } = useTranslation();
    const { setTheme } = useTheme();
    const navigate = useNavigate();
    const location = useLocation();
    const page = useRoutes(routes);

    useEffect(() => {
        if (appWindow.label === 'config') {
            appWindow.show();
        }
    }, []);

    const current = TABS.find((k) => location.pathname.startsWith(`/${k}`)) ?? 'translate';

    return (
        <div
            className={`h-screen flex flex-col bg-background select-none cursor-default ${
                osType === 'Linux' && 'rounded-[10px] border-1 border-default-100'
            }`}
        >
            {/* 整个 config 窗口只能有这一个 Toaster。react-hot-toast 的自动消失定时器
                和全局 pausedAt 都挂在挂载着的 Toaster 上：把它放进各个 tab / 各个服务
                的 Config 里，切个 tab 就把定时器清了，那条 toast 会永远留在全局 store
                里，之后每挂载一个新 Toaster 就再弹一次。 */}
            <Toaster
                position='top-center'
                containerStyle={{ top: 45 }}
            />
            {/* 第一层：窗口最外层。图标 + 名字 / 主题 + 语言 / 最小化最大化关闭都在这条。 */}
            <div
                data-tauri-drag-region='true'
                className='h-[35px] shrink-0 flex items-center gap-[8px] pl-[10px] bg-content3'
            >
                <img
                    data-tauri-drag-region='true'
                    alt='logo'
                    src='icon.svg'
                    className='h-[20px] w-[20px]'
                    draggable={false}
                />
                <span
                    data-tauri-drag-region='true'
                    className='font-bold text-[13px]'
                >
                    TiLex
                </span>
                <div
                    data-tauri-drag-region='true'
                    className='grow h-full'
                />
                {appTheme !== null && (
                    <Tooltip content={t(`config.general.theme.${appTheme}`)}>
                        <Button
                            isIconOnly
                            size='sm'
                            radius='full'
                            variant='light'
                            aria-label='theme'
                            className='w-[26px] h-[26px] min-w-[26px]'
                            onPress={() => {
                                const next = THEME_CYCLE[appTheme] ?? 'system';
                                setAppTheme(next);
                                // App.jsx 收到 app_theme_changed 会自己应用，包括跟随系统那套监听。
                                if (next !== 'system') setTheme(next);
                            }}
                        >
                            {THEME_ICON[appTheme]}
                        </Button>
                    </Tooltip>
                )}
                {appLanguage !== null && (
                    <Button
                        isIconOnly
                        size='sm'
                        radius='full'
                        variant='light'
                        aria-label='language'
                        className='w-[26px] h-[26px] min-w-[26px] font-bold text-[12px] mr-[4px]'
                        onPress={() => {
                            const next = appLanguage === 'zh_cn' ? 'en' : 'zh_cn';
                            setAppLanguage(next);
                            i18n.changeLanguage(next);
                            invoke('update_tray', { language: next, copyMode: '' });
                        }}
                    >
                        {appLanguage === 'zh_cn' ? '中' : 'EN'}
                    </Button>
                )}
                {osType !== 'Darwin' && <WindowControl />}
            </div>
            {/* 第二层：只放 Tab，居中。 */}
            <div
                data-tauri-drag-region='true'
                className='h-[52px] shrink-0 flex items-center justify-center border-b-1 border-default-200'
            >
                <Tabs
                    size='sm'
                    radius='full'
                    selectedKey={current}
                    onSelectionChange={(key) => navigate(`/${key}`)}
                    aria-label='config tabs'
                >
                    {TABS.map((key) => (
                        <Tab
                            key={key}
                            title={t(`config.${key}.label`)}
                        />
                    ))}
                </Tabs>
            </div>
            <div className='grow overflow-y-auto p-[10px]'>{page}</div>
        </div>
    );
}
