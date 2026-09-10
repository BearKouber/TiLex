import { enable, isEnabled, disable } from 'tauri-plugin-autostart-api';
import { appLogDir, appConfigDir } from '@tauri-apps/api/path';
import { Divider, Button, Card, CardBody, Switch } from '@nextui-org/react';
import React, { useEffect, useState } from 'react';
import { useTranslation, Trans } from 'react-i18next';
import { open } from '@tauri-apps/api/shell';
import { fetch, ResponseType } from '@tauri-apps/api/http';
import { info } from 'tauri-plugin-log-api';
import toast from 'react-hot-toast';

import { appVersion } from '../../../../utils/env';
import { useToastStyle } from '../../../../hooks';

const REPO = 'https://github.com/BearKouber/TiLex';
// 对应 config.about.credits 里的 <0>…<3>
const CREDIT_LINKS = [
    'https://github.com/pot-app/pot-desktop',
    'https://github.com/tisfeng/Easydict',
    'https://github.com/STranslate/STranslate',
    `${REPO}/blob/HEAD/LICENSE`,
];

// "v0.2.0" 比 "0.1.0" 新吗。parseInt 吃掉 "0-beta" 这种后缀。
const isNewer = (tag, cur) => {
    const a = tag.replace(/^v/, '').split('.');
    const b = cur.replace(/^v/, '').split('.');
    for (let i = 0; i < 3; i++) {
        const x = parseInt(a[i]) || 0;
        const y = parseInt(b[i]) || 0;
        if (x !== y) return x > y;
    }
    return false;
};

// 开机自启放这里：它和「查看日志 / 查看配置文件」一样是应用级的杂项，
// 不属于翻译、服务、生词本任何一页。顶栏只放视图开关（主题 / 语言），不放持久设置。
export default function About() {
    const [autoStart, setAutoStart] = useState(false);
    const [checking, setChecking] = useState(false);
    const { t } = useTranslation();
    const toastStyle = useToastStyle();

    useEffect(() => {
        isEnabled().then(setAutoStart);
    }, []);

    // 只告知，不下载不安装：TiLex 不做自动更新。
    // 不走 api.github.com：未登录每 IP 每小时 60 次，共用出口 IP / 代理很容易 403。
    // 网页的 releases/latest 没这个限制，有发布时重定向到 /releases/tag/<版本>，
    // 没发布时落到 /releases 或 404。tauri 的 fetch 跟随重定向，res.url 是最终地址。
    const checkUpdate = async () => {
        setChecking(true);
        try {
            const res = await fetch(`${REPO}/releases/latest`, {
                method: 'HEAD',
                timeout: 10,
                responseType: ResponseType.Text,
            });
            const m = res.url.match(/\/releases\/tag\/([^/?#]+)$/);
            const tag = m && decodeURIComponent(m[1]);
            if (!tag && (res.ok || res.status === 404)) {
                toast(t('config.about.no_release'), { style: toastStyle });
            } else if (!res.ok) {
                throw new Error(`HTTP ${res.status}`);
            } else if (isNewer(tag, appVersion)) {
                toast.success(t('config.about.new_version', { version: tag }), {
                    style: toastStyle,
                    duration: 6000,
                });
            } else {
                toast.success(t('config.about.up_to_date'), { style: toastStyle });
            }
        } catch (e) {
            toast.error(t('config.about.check_failed', { error: e.message ?? String(e) }), { style: toastStyle });
        } finally {
            setChecking(false);
        }
    };

    return (
        <div className='h-full w-full py-[40px] px-[100px]'>
            <img
                src='icon.png'
                className='mx-auto h-[80px] mb-[5px]'
                draggable={false}
            />
            <div className='content-center'>
                <h1 className='font-bold text-2xl text-center'>TiLex</h1>
                <p className='text-center text-sm text-gray-500'>{appVersion}</p>
                <p className='text-center text-sm mt-[5px]'>{t('config.about.description')}</p>
                <div className='flex justify-center gap-2 my-[10px]'>
                    <Button
                        variant='light'
                        size='sm'
                        onPress={() => open(REPO)}
                    >
                        {t('config.about.github')}
                    </Button>
                    <Button
                        variant='light'
                        size='sm'
                        onPress={() => open(`${REPO}/issues`)}
                    >
                        {t('config.about.feedback')}
                    </Button>
                    <Button
                        variant='light'
                        size='sm'
                        onPress={() => open(`${REPO}/releases`)}
                    >
                        {t('config.about.releases')}
                    </Button>
                </div>
                <Divider />
            </div>
            <Card
                shadow='none'
                className='mt-[20px] border-1 border-default-200'
            >
                <CardBody>
                    <div className='config-item'>
                        <h3 className='my-auto'>{t('config.general.auto_start')}</h3>
                        <Switch
                            isSelected={autoStart}
                            onValueChange={(v) => {
                                setAutoStart(v);
                                (v ? enable() : disable()).then(() => {
                                    info(`Auto start ${v ? 'enabled' : 'disabled'}`);
                                });
                            }}
                        />
                    </div>
                </CardBody>
            </Card>
            <div className='flex justify-center gap-6 mt-[20px]'>
                <Button
                    variant='bordered'
                    size='sm'
                    isLoading={checking}
                    onPress={checkUpdate}
                >
                    {t('config.about.check_update')}
                </Button>
                <Button
                    variant='bordered'
                    size='sm'
                    onPress={async () => open(await appLogDir())}
                >
                    {t('config.about.view_log')}
                </Button>
                <Button
                    variant='bordered'
                    size='sm'
                    onPress={async () => open(await appConfigDir())}
                >
                    {t('config.about.view_config')}
                </Button>
            </div>
            {/* GPL-3 §5(a) 要求声明「改过」；顺带给出协议全文入口。HEAD 指默认分支，不怕分支改名。
                中英文语序不同（GPL-3.0 在「开源协议」后 / 在 "License" 前），所以链接位置写在文案里。 */}
            <p className='text-center text-xs text-gray-500 mt-[30px]'>
                <Trans
                    i18nKey='config.about.credits'
                    components={CREDIT_LINKS.map((url) => (
                        <span
                            key={url}
                            className='cursor-pointer hover:underline'
                            onClick={() => open(url)}
                        />
                    ))}
                />
            </p>
        </div>
    );
}
