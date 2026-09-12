import { Modal, ModalContent, ModalHeader, ModalBody, ModalFooter } from '@nextui-org/react';
import { DragDropContext, Draggable, Droppable } from 'react-beautiful-dnd';
import { Card, Spacer, Button, useDisclosure } from '@nextui-org/react';
import toast from 'react-hot-toast';
import { useTranslation } from 'react-i18next';
import React, { useState } from 'react';

import { useToastStyle } from '../../../../../hooks';
import { useConfig } from '../../../../../hooks';
import { config as configStore } from '../../../../../utils/store';
import SelectModal from './SelectModal';
import SelectAiModal from './SelectAiModal';
import ServiceItem from './ServiceItem';
import ConfigModal from './ConfigModal';

export default function Translate() {
    const { isOpen: isConfigOpen, onOpen: onConfigOpen, onOpenChange: onConfigOpenChange } = useDisclosure();
    const { isOpen: isAddOpen, onOpen: onAddOpen, onOpenChange: onAddOpenChange } = useDisclosure();
    const {
        isOpen: isSelectBuiltinOpen,
        onOpen: onSelectBuiltinOpen,
        onOpenChange: onSelectBuiltinOpenChange,
    } = useDisclosure();
    const { isOpen: isSelectAiOpen, onOpen: onSelectAiOpen, onOpenChange: onSelectAiOpenChange } = useDisclosure();
    // 选中的厂商预设。null = 自定义（空实例），配置弹窗关掉就该清干净。
    const [presetConfig, setPresetConfig] = useState(null);
    const [adding, setAdding] = useState(false);
    const [currentConfigKey, setCurrentConfigKey] = useState('google');
    // now it's service instance list
    const [translateServiceInstanceList, setTranslateServiceInstanceList] = useConfig('translate_service_list', [
        'google',
    ]);

    const { t } = useTranslation();
    const toastStyle = useToastStyle();

    const reorder = (list, startIndex, endIndex) => {
        const result = Array.from(list);
        const [removed] = result.splice(startIndex, 1);
        result.splice(endIndex, 0, removed);
        return result;
    };
    const onDragEnd = async (result) => {
        if (!result.destination) return;
        const items = reorder(translateServiceInstanceList, result.source.index, result.destination.index);
        setTranslateServiceInstanceList(items);
    };

    const deleteServiceInstance = async (instanceKey) => {
        if ((configStore.value('translate_service_list') ?? []).length <= 1) {
            toast.error(t('config.service.least'), { style: toastStyle });
            return;
        }
        try {
            await configStore.removeService('translate_service_list', instanceKey);
        } catch {
            toast.error(t('config.save_failed'), { style: toastStyle });
        }
    };
    const updateServiceInstanceList = (instanceKey, value) =>
        configStore.saveService('translate_service_list', instanceKey, value, instanceKey, adding);

    return (
        <>
            {/* 高度不再自己算：外层 Service/index.jsx 是 flex-col，这段吃剩下的
                空间。min-h-0 是 flex item 能收缩到内容高度以下的前提。 */}
            <Card
                shadow='none'
                className='flex-1 min-h-0 overflow-y-auto p-5 flex justify-between border-1 border-default-200'
            >
                <DragDropContext onDragEnd={onDragEnd}>
                    <Droppable
                        droppableId='droppable'
                        direction='vertical'
                    >
                        {(provided) => (
                            <div
                                className='overflow-y-auto h-full'
                                ref={provided.innerRef}
                                {...provided.droppableProps}
                            >
                                {translateServiceInstanceList !== null &&
                                    translateServiceInstanceList.map((x, i) => {
                                        return (
                                            <Draggable
                                                key={x}
                                                draggableId={x}
                                                index={i}
                                            >
                                                {(provided) => {
                                                    return (
                                                        <div
                                                            ref={provided.innerRef}
                                                            {...provided.draggableProps}
                                                        >
                                                            <ServiceItem
                                                                {...provided.dragHandleProps}
                                                                key={x}
                                                                serviceInstanceKey={x}
                                                                deleteServiceInstance={deleteServiceInstance}
                                                                setCurrentConfigKey={(key) => {
                                                                    setAdding(false);
                                                                    setPresetConfig(null);
                                                                    setCurrentConfigKey(key);
                                                                }}
                                                                onConfigOpen={onConfigOpen}
                                                            />
                                                            <Spacer y={2} />
                                                        </div>
                                                    );
                                                }}
                                            </Draggable>
                                        );
                                    })}
                            </div>
                        )}
                    </Droppable>
                </DragDropContext>
                <Spacer y={2} />
                {/* 两条路：内置引擎（谷歌 / 必应 / 百度…）和自填 OpenAI 兼容地址。
                    内置那张列表是从批次 4 删掉的 SelectModal 捞回来的 ——
                    没有它，谷歌一删就再也加不回来。 */}
                <Button
                    fullWidth
                    variant='bordered'
                    onPress={onAddOpen}
                >
                    {t('config.service.add_service')}
                </Button>
            </Card>
            <Modal
                isOpen={isAddOpen}
                onOpenChange={onAddOpenChange}
            >
                <ModalContent>
                    {(onClose) => (
                        <>
                            <ModalHeader>{t('config.service.add_service')}</ModalHeader>
                            <ModalBody>
                                <Button
                                    fullWidth
                                    variant='bordered'
                                    onPress={() => {
                                        onClose();
                                        onSelectBuiltinOpen();
                                    }}
                                >
                                    {t('config.service.add_builtin_service')}
                                </Button>
                                <Button
                                    fullWidth
                                    variant='bordered'
                                    onPress={() => {
                                        onClose();
                                        onSelectAiOpen();
                                    }}
                                >
                                    {t('config.service.add_ai_service')}
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
            <SelectAiModal
                isOpen={isSelectAiOpen}
                onOpenChange={onSelectAiOpenChange}
                setCurrentConfigKey={(key) => {
                    setAdding(true);
                    setCurrentConfigKey(key);
                }}
                setPresetConfig={setPresetConfig}
                onConfigOpen={onConfigOpen}
            />
            <SelectModal
                isOpen={isSelectBuiltinOpen}
                onOpenChange={onSelectBuiltinOpenChange}
                setCurrentConfigKey={(key) => {
                    setAdding(true);
                    setPresetConfig(null);
                    setCurrentConfigKey(key);
                }}
                onConfigOpen={onConfigOpen}
            />
            {/* key 换实例就重挂：useConfig 的 key 和默认值都是挂载时捕获的，
                不重挂的话预设填不进去，还会写到上一个实例的配置里。 */}
            {isConfigOpen && (
                <ConfigModal
                    key={currentConfigKey}
                    serviceInstanceKey={currentConfigKey}
                    presetConfig={presetConfig}
                    isOpen={isConfigOpen}
                    onOpenChange={onConfigOpenChange}
                    updateServiceInstanceList={updateServiceInstanceList}
                />
            )}
        </>
    );
}
