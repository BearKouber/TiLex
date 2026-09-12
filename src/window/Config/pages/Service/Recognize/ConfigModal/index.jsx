import { Button, Input, Modal, ModalBody, ModalContent, ModalFooter, ModalHeader } from '@nextui-org/react';
import React, { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { invoke } from '@tauri-apps/api/tauri';
import { open } from '@tauri-apps/api/shell';
import toast from 'react-hot-toast';

import { RECOGNIZE_SERVICES, recognizeConfigKey } from '../../../../../../utils/recognize';
import { postBase64 } from '../../../../../../services/recognize/umi';
import { useConfig, useToastStyle } from '../../../../../../hooks';

// 1x1 的白点，只是拿来敲一下端口看通不通。认不出字（code 101）也算连上了。
const PING_PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';

// 两家 OCR 的识别能力都来自第三方开源项目，来源和用途在弹窗里如实写清楚。
function SourceCard({ repo, text }) {
    return (
        <div className='rounded-md bg-content2 px-[10px] py-[8px] text-[12px] text-default-500'>
            {text}
            <span
                className='ml-[4px] text-primary cursor-pointer'
                onClick={() => open(`https://github.com/${repo}`)}
            >
                {repo}
            </span>
        </div>
    );
}

// 微信 OCR 没有可配项 —— 路径两条都是 Rust 侧自动探测的（src-tauri/src/ocr.rs），
// 所以这个弹窗只有「探到了什么」和一个重探按钮。
function WechatBody({ t }) {
    const [status, setStatus] = useState(null);
    const toastStyle = useToastStyle();

    const probe = (notify) =>
        invoke('ocr_status').then(
            (version) => {
                setStatus({ version });
                notify && toast.success(t('config.service.test_success'), { style: toastStyle });
            },
            (error) => {
                setStatus({ error: String(error) });
                notify && toast.error(String(error), { style: toastStyle });
            }
        );

    useEffect(() => {
        void probe(false);
    }, []);

    return (
        <>
            <div className='config-item'>
                <h3 className='my-auto'>{t('config.service.wechat_version')}</h3>
                <span className={`my-auto text-[13px] ${status?.error ? 'text-danger' : 'text-default-500'}`}>
                    {status === null ? '…' : status.error ?? status.version}
                </span>
            </div>
            <p className='text-[12px] text-default-400'>{t('config.service.wechat_hint')}</p>
            <SourceCard
                repo='swigger/wechat-ocr'
                text={t('config.service.wechat_source')}
            />
            <Button
                variant='flat'
                onPress={() => probe(true)}
            >
                {t('config.service.test')}
            </Button>
        </>
    );
}

function UmiBody({ t, draft, setDraft }) {
    const toastStyle = useToastStyle();
    const [testing, setTesting] = useState(false);

    const test = async () => {
        setTesting(true);
        try {
            await postBase64(draft.url, PING_PNG);
            toast.success(t('config.service.test_success'), { style: toastStyle });
        } catch (e) {
            toast.error(String(e.message ?? e), { style: toastStyle });
        } finally {
            setTesting(false);
        }
    };

    return (
        <>
            <div className='config-item'>
                <h3 className='my-auto'>{t('config.service.umi_url')}</h3>
                <Input
                    aria-label='url'
                    variant='bordered'
                    className='w-[60%]'
                    value={draft.url ?? ''}
                    onValueChange={(url) => setDraft({ ...draft, url })}
                />
            </div>
            <p className='text-[12px] text-default-400'>{t('config.service.umi_hint')}</p>
            <SourceCard
                repo='hiroi-sora/Umi-OCR'
                text={t('config.service.umi_source')}
            />
            <Button
                variant='flat'
                isLoading={testing}
                onPress={test}
            >
                {t('config.service.test')}
            </Button>
        </>
    );
}

export default function ConfigModal(props) {
    const { name, isOpen, onOpenChange, updateServiceList } = props;
    const [config] = useConfig(recognizeConfigKey(name), {}, { sync: false });
    const [draft, setDraft] = useState(null);
    const [saving, setSaving] = useState(false);
    const { t } = useTranslation();

    // 打开时拷一份草稿，取消就整份丢掉，不去动已存的配置。
    useEffect(() => {
        if (isOpen && config !== null) {
            setDraft({ ...RECOGNIZE_SERVICES[name].defaultConfig, ...config });
        }
    }, [isOpen, name, config !== null]);

    if (!RECOGNIZE_SERVICES[name]) return <></>;

    return (
        <Modal
            isOpen={isOpen}
            onOpenChange={onOpenChange}
        >
            <ModalContent>
                {(onClose) =>
                    draft && (
                        <>
                            <ModalHeader>{t(`config.service.${name}_ocr`)}</ModalHeader>
                            <ModalBody>
                                {name === 'wechat' ? (
                                    <WechatBody t={t} />
                                ) : (
                                    <UmiBody
                                        t={t}
                                        draft={draft}
                                        setDraft={setDraft}
                                    />
                                )}
                            </ModalBody>
                            <ModalFooter>
                                <Button
                                    variant='light'
                                    onPress={onClose}
                                >
                                    {t('common.cancel')}
                                </Button>
                                <Button
                                    color='primary'
                                    isLoading={saving}
                                    onPress={async () => {
                                        setSaving(true);
                                        try {
                                            await updateServiceList(name, { ...config, ...draft });
                                            onClose();
                                        } catch {
                                            toast.error(t('config.save_failed'));
                                        } finally {
                                            setSaving(false);
                                        }
                                    }}
                                >
                                    {t('common.ok')}
                                </Button>
                            </ModalFooter>
                        </>
                    )
                }
            </ModalContent>
        </Modal>
    );
}
