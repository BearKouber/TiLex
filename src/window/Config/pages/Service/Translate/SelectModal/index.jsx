import { Modal, ModalContent, ModalHeader, ModalBody, ModalFooter, Button } from '@nextui-org/react';
import { useTranslation } from 'react-i18next';
import React from 'react';

import * as builtinServices from '../../../../../../services/translate';
import { createServiceInstanceKey } from '../../../../../../utils/service_instance';

export default function SelectModal(props) {
    const { isOpen, onOpenChange, setCurrentConfigKey, onConfigOpen } = props;
    const { t } = useTranslation();

    return (
        <Modal
            isOpen={isOpen}
            onOpenChange={onOpenChange}
            scrollBehavior='inside'
        >
            <ModalContent className='max-h-[80vh]'>
                {(onClose) => (
                    <>
                        <ModalHeader>{t('config.service.add_builtin_service')}</ModalHeader>
                        <ModalBody>
                            {/* ai 不列在这儿 —— 上一层的「添加 AI 服务」
                                就是它，列两遍等于同一个东西给两个入口。 */}
                            {Object.keys(builtinServices)
                                .filter((x) => x !== 'ai')
                                .map((x) => (
                                    <div key={x}>
                                        <Button
                                            fullWidth
                                            onPress={() => {
                                                setCurrentConfigKey(createServiceInstanceKey(x));
                                                onConfigOpen();
                                                onClose();
                                            }}
                                            startContent={
                                                <img
                                                    src={builtinServices[x].info.icon}
                                                    className='h-[24px] w-[24px] my-auto'
                                                />
                                            }
                                        >
                                            <div className='w-full'>
                                                {t(`services.translate.${builtinServices[x].info.name}.title`)}
                                            </div>
                                        </Button>
                                    </div>
                                ))}
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
