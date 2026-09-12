import {
    Button,
    Card,
    Modal,
    ModalBody,
    ModalContent,
    ModalFooter,
    ModalHeader,
    Spacer,
    useDisclosure,
} from '@nextui-org/react';
import { DragDropContext, Draggable, Droppable } from 'react-beautiful-dnd';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';

import {
    DEFAULT_RECOGNIZE_LIST,
    RECOGNIZE_LIST_KEY,
    RECOGNIZE_SERVICES,
    recognizeConfigKey,
} from '../../../../../utils/recognize';
import { useConfig } from '../../../../../hooks';
import toast from 'react-hot-toast';
import { config as configStore } from '../../../../../utils/store';
import ServiceItem from './ServiceItem';
import ConfigModal from './ConfigModal';

// 文本识别的服务列表。交互和翻译那边刻意做成一样的（拖拽 / 开关 / 编辑 / 删除 /
// 添加），差别只在数据模型：识别没有插件、没有实例 key，列表元素就是服务名。
// 顺序即优先级 —— 识别只取第一个启用的返回。

export default function Recognize() {
    const { isOpen: isConfigOpen, onOpen: onConfigOpen, onOpenChange: onConfigOpenChange } = useDisclosure();
    const { isOpen: isAddOpen, onOpen: onAddOpen, onOpenChange: onAddOpenChange } = useDisclosure();
    const [currentConfigKey, setCurrentConfigKey] = useState('wechat');
    const [adding, setAdding] = useState(false);
    const [serviceList, setServiceList] = useConfig(RECOGNIZE_LIST_KEY, DEFAULT_RECOGNIZE_LIST);
    const { t } = useTranslation();

    const onDragEnd = (result) => {
        if (!result.destination) return;
        const items = Array.from(serviceList);
        const [removed] = items.splice(result.source.index, 1);
        items.splice(result.destination.index, 0, removed);
        setServiceList(items);
    };

    // 不拦「至少留一个」：一个都不剩时上层截图那条路已经有 no_recognize 的提示了。
    const deleteService = async (name) => {
        try {
            await configStore.removeService(RECOGNIZE_LIST_KEY, recognizeConfigKey(name), name);
        } catch {
            toast.error(t('config.save_failed'));
        }
    };

    const updateServiceList = (name, value) =>
        configStore.saveService(RECOGNIZE_LIST_KEY, recognizeConfigKey(name), value, name, adding);

    // 还没装上的服务，就是「添加服务」弹窗里能挑的那些。
    const addable = Object.keys(RECOGNIZE_SERVICES).filter((x) => !(serviceList ?? []).includes(x));

    return (
        <>
            <Card
                shadow='none'
                className='flex-1 min-h-0 overflow-y-auto p-5 flex justify-between border-1 border-default-200'
            >
                <DragDropContext onDragEnd={onDragEnd}>
                    <Droppable
                        droppableId='recognize-droppable'
                        direction='vertical'
                    >
                        {(provided) => (
                            <div
                                className='overflow-y-auto h-full'
                                ref={provided.innerRef}
                                {...provided.droppableProps}
                            >
                                {serviceList !== null &&
                                    serviceList.map((x, i) => (
                                        <Draggable
                                            key={x}
                                            draggableId={x}
                                            index={i}
                                        >
                                            {(provided) => (
                                                <div
                                                    ref={provided.innerRef}
                                                    {...provided.draggableProps}
                                                >
                                                    <ServiceItem
                                                        {...provided.dragHandleProps}
                                                        name={x}
                                                        deleteService={deleteService}
                                                        setCurrentConfigKey={(name) => {
                                                            setAdding(false);
                                                            setCurrentConfigKey(name);
                                                        }}
                                                        onConfigOpen={onConfigOpen}
                                                    />
                                                    <Spacer y={2} />
                                                </div>
                                            )}
                                        </Draggable>
                                    ))}
                                {provided.placeholder}
                            </div>
                        )}
                    </Droppable>
                </DragDropContext>
                <Spacer y={2} />
                {/* 不加 isDisabled：两家都装上时按钮会变灰，和「文本翻译」那边
                    同一个按钮长得不一样。装满了就在弹窗里说一句。 */}
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
                                {addable.length === 0 && (
                                    <p className='text-sm text-default-500'>{t('config.service.all_installed')}</p>
                                )}
                                {addable.map((name) => (
                                    <Button
                                        key={name}
                                        fullWidth
                                        variant='bordered'
                                        onPress={() => {
                                            onClose();
                                            setAdding(true);
                                            setCurrentConfigKey(name);
                                            onConfigOpen();
                                        }}
                                    >
                                        {t(`config.service.${name}_ocr`)}
                                    </Button>
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
            {/* key 换服务就重挂：useConfig 的 key 是挂载时捕获的，不重挂会写到上
                一个服务的配置里去。 */}
            {isConfigOpen && (
                <ConfigModal
                    key={currentConfigKey}
                    name={currentConfigKey}
                    isOpen={isConfigOpen}
                    onOpenChange={onConfigOpenChange}
                    updateServiceList={updateServiceList}
                />
            )}
        </>
    );
}
