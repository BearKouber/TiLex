import { Modal, ModalContent, ModalHeader, ModalBody, ModalFooter, Button } from '@nextui-org/react';
import { useTranslation } from 'react-i18next';
import React from 'react';

import { AI_PRESETS } from '../../../../../../services/translate/ai/presets';
import { ServiceIcon } from '../../../../../../services/translate/ai/ServiceIcon';
import { createServiceInstanceKey } from '../../../../../../utils/service_instance';

// 「添加 AI 服务」原来是直接建一个空的 ai 实例，地址 / 格式 / 模型全空。
// 这里先让用户挑一家，把那三样连同图标一起预填进去，剩下就只有粘 Key。
//
// 预设走 presetConfig 这个 prop 传给配置弹窗，当 useConfig 的默认值用：实例
// key 是新的，store 里读不到，默认值就是最终值。不自己往 store 里写一遍。
export default function SelectAiModal(props) {
    const { isOpen, onOpenChange, setCurrentConfigKey, setPresetConfig, onConfigOpen } = props;
    const { t } = useTranslation();

    const pick = (preset, onClose) => {
        onClose();
        setPresetConfig(preset);
        setCurrentConfigKey(createServiceInstanceKey('ai'));
        onConfigOpen();
    };

    return (
        <Modal
            isOpen={isOpen}
            onOpenChange={onOpenChange}
            scrollBehavior='inside'
        >
            <ModalContent className='max-h-[80vh]'>
                {(onClose) => (
                    <>
                        <ModalHeader>{t('config.service.add_ai_service')}</ModalHeader>
                        <ModalBody>
                            <div className='grid grid-cols-3 gap-2'>
                                {AI_PRESETS.map((preset) => (
                                    <Button
                                        key={preset.id}
                                        variant='bordered'
                                        className='h-[64px] flex-col gap-1 px-1'
                                        onPress={() => pick(preset, onClose)}
                                    >
                                        <ServiceIcon id={preset.id} />
                                        <span className='text-[11px] truncate w-full'>{preset.name}</span>
                                    </Button>
                                ))}
                            </div>
                            {/* 自建反代 / OneAPI / NewAPI / 本地中转都走这条 */}
                            <Button
                                fullWidth
                                variant='bordered'
                                className='h-[56px]'
                                onPress={() => pick(null, onClose)}
                            >
                                <div className='w-full text-left'>
                                    <div className='text-[13px]'>{t('config.service.custom_ai_service')}</div>
                                    <div className='text-[10px] text-default-500'>
                                        {t('config.service.custom_ai_service_desc')}
                                    </div>
                                </div>
                            </Button>
                        </ModalBody>
                        <ModalFooter>
                            <Button
                                color='danger'
                                variant='light'
                                onPress={onClose}
                            >
                                {t('common.cancel')}
                            </Button>
                        </ModalFooter>
                    </>
                )}
            </ModalContent>
        </Modal>
    );
}
