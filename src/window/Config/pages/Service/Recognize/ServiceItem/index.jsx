import { RxDragHandleHorizontal } from 'react-icons/rx';
import { Spacer, Button, Switch } from '@nextui-org/react';
import { MdDeleteOutline } from 'react-icons/md';
import { TbTextRecognition } from 'react-icons/tb';
import { RiWechatFill } from 'react-icons/ri';
import { useTranslation } from 'react-i18next';
import { BiSolidEdit } from 'react-icons/bi';
import React, { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/tauri';

import { RECOGNIZE_SERVICES, recognizeConfigKey } from '../../../../../../utils/recognize';
import { useConfig } from '../../../../../../hooks';

// 一行识别服务。**类名逐条对齐 Translate/ServiceItem** —— 「右侧控件在同一条垂线上」
// 靠的就是两边用同一套 padding / size，不是看着差不多。改这里之前先去看那份。
//
// 图标不用 svg 资源：识别这两家没有官方 logo 文件，用已经装了的 react-icons，
// 外面套同样的 h-[24px] w-[24px] 盒子，占位和翻译那边的 <img> 一致。
const ICONS = {
    wechat: <RiWechatFill className='h-[24px] w-[24px] my-auto text-[#07C160]' />,
    umi: <TbTextRecognition className='h-[24px] w-[24px] my-auto text-default-600' />,
};

// 微信那行的副标题：探测到的微信版本，或者「缺什么」的说明。
function WechatStatus() {
    const [status, setStatus] = useState(null);
    useEffect(() => {
        invoke('ocr_status').then(
            (version) => setStatus({ version }),
            (error) => setStatus({ error: String(error) })
        );
    }, []);
    if (status === null) return <span className='text-[12px] text-default-400 my-auto'>…</span>;
    return (
        <span className={`text-[12px] my-auto ${status.error ? 'text-danger' : 'text-default-400'}`}>
            {status.error ?? status.version}
        </span>
    );
}

export default function ServiceItem(props) {
    const { name, deleteService, setCurrentConfigKey, onConfigOpen, ...drag } = props;
    const { t } = useTranslation();
    const [config, setConfig] = useConfig(recognizeConfigKey(name), {});

    // 列表存在 config.json 里，可能留着已经删掉的服务名（老版本的 google）。
    // 拿不到实现就整条不渲染，别让一个陈旧的 key 把整页炸掉 —— 配置窗是无边框
    // 透明窗，React 一崩看到的是「整个窗口没了」，不是白屏。
    if (!RECOGNIZE_SERVICES[name]) return <></>;

    return (
        config !== null && (
            <div className='bg-content2 rounded-md px-[10px] py-[20px] flex justify-between'>
                <div className='flex'>
                    <div
                        {...drag}
                        className='text-2xl my-auto'
                    >
                        <RxDragHandleHorizontal />
                    </div>
                    <Spacer x={2} />
                    {ICONS[name]}
                    <Spacer x={2} />
                    <h2 className='my-auto'>{t(`config.service.${name}_ocr`)}</h2>
                    {name === 'wechat' && (
                        <>
                            <Spacer x={2} />
                            <WechatStatus />
                        </>
                    )}
                </div>
                <div className='flex'>
                    <Switch
                        size='sm'
                        isSelected={config['enable'] ?? true}
                        onValueChange={(v) => {
                            setConfig({ ...config, enable: v });
                        }}
                    />
                    <Button
                        isIconOnly
                        size='sm'
                        variant='light'
                        onPress={() => {
                            setCurrentConfigKey(name);
                            onConfigOpen();
                        }}
                    >
                        <BiSolidEdit className='text-2xl' />
                    </Button>
                    <Spacer x={2} />
                    <Button
                        isIconOnly
                        size='sm'
                        variant='light'
                        color='danger'
                        onPress={() => {
                            deleteService(name);
                        }}
                    >
                        <MdDeleteOutline className='text-2xl' />
                    </Button>
                </div>
            </div>
        )
    );
}
