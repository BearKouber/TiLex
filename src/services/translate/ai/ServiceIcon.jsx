import { SiAlibabacloud, SiAnthropic, SiGithubcopilot, SiGooglegemini, SiXiaomi } from 'react-icons/si';
import { Modal, ModalContent, ModalHeader, ModalBody, Button } from '@nextui-org/react';
import { MdAutoAwesome } from 'react-icons/md';
import { useTranslation } from 'react-i18next';
import React from 'react';

import { ICONS, ICON_IDS, FALLBACK_ICON } from './icons';

// react-icons 5.3 里有的那几家。剩下的走 icons.js 里的字母方块。
const COMPONENTS = {
    claude: SiAnthropic,
    gemini: SiGooglegemini,
    qwen: SiAlibabacloud,
    copilot: SiGithubcopilot,
    mimo: SiXiaomi,
    sparkle: MdAutoAwesome,
};

// 三种渲染路径都塞进同一个 24x24 盒子里 —— 服务列表那一行的图标位是
// h-[24px] w-[24px] 的 <img>，尺寸不一致右边一整列就对不齐了。
export function ServiceIcon({ id, className = 'h-[24px] w-[24px]' }) {
    const key = id in ICONS ? id : FALLBACK_ICON;
    const meta = ICONS[key];
    if (meta.file) {
        return (
            <img
                src={meta.file}
                alt={meta.label}
                className={`${className} my-auto shrink-0`}
                draggable={false}
            />
        );
    }
    const Comp = COMPONENTS[key];
    if (Comp) {
        return (
            <Comp
                className={`${className} my-auto shrink-0`}
                style={{ color: meta.color }}
            />
        );
    }
    return (
        <div
            className={`${className} my-auto shrink-0 rounded-md flex items-center justify-center text-white text-[13px] leading-none`}
            style={{ background: meta.color }}
        >
            {meta.letter}
        </div>
    );
}

export function IconPickerModal(props) {
    const { isOpen, onOpenChange, onPick } = props;
    const { t } = useTranslation();

    return (
        <Modal
            isOpen={isOpen}
            onOpenChange={onOpenChange}
            scrollBehavior='inside'
        >
            <ModalContent className='max-h-[70vh]'>
                {(onClose) => (
                    <>
                        <ModalHeader>{t('services.translate.ai.pick_icon')}</ModalHeader>
                        <ModalBody className='pb-5'>
                            <div className='grid grid-cols-4 gap-2'>
                                {ICON_IDS.map((id) => (
                                    <Button
                                        key={id}
                                        variant='bordered'
                                        className='h-[56px] flex-col gap-1'
                                        onPress={() => {
                                            onPick(id);
                                            onClose();
                                        }}
                                    >
                                        <ServiceIcon id={id} />
                                        <span className='text-[10px] text-default-500 truncate w-full'>
                                            {ICONS[id].label}
                                        </span>
                                    </Button>
                                ))}
                            </div>
                        </ModalBody>
                    </>
                )}
            </ModalContent>
        </Modal>
    );
}
