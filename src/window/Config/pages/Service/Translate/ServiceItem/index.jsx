import { RxDragHandleHorizontal } from 'react-icons/rx';
import { Spacer, Button, Switch } from '@nextui-org/react';
import { MdDeleteOutline } from 'react-icons/md';
import { useTranslation } from 'react-i18next';
import { BiSolidEdit } from 'react-icons/bi';
import React from 'react';

import * as builtinServices from '../../../../../../services/translate';
import { ServiceIcon } from '../../../../../../services/translate/ai/ServiceIcon';
import { matchIcon } from '../../../../../../services/translate/ai/icons';
import { useConfig } from '../../../../../../hooks';
import {
    INSTANCE_NAME_CONFIG_KEY,
    getDisplayInstanceName,
    getServiceName,
    whetherAvailableService,
} from '../../../../../../utils/service_instance';

export default function ServiceItem(props) {
    const { serviceInstanceKey, deleteServiceInstance, setCurrentConfigKey, onConfigOpen, ...drag } = props;
    const { t } = useTranslation();
    const [serviceInstanceConfig, setServiceInstanceConfig] = useConfig(serviceInstanceKey, {});

    const serviceName = getServiceName(serviceInstanceKey);

    // AI 实例全是 ai，共用一张绿色 logo 的话装三家就分不出谁是谁。
    // 没存过 icon 的老实例按地址 / 模型现猜一个，不用点进去改一遍。
    const aiIcon =
        serviceName === 'ai'
            ? serviceInstanceConfig?.icon || matchIcon(serviceInstanceConfig?.requestPath, serviceInstanceConfig?.model)
            : null;

    // 内置服务也会消失（批次 4 删掉了 15 个），而 translate_service_list 是存在
    // config.json 里的，还留着老名字。拿不到实现就整条不渲染，别让一个陈旧的
    // key 把整页炸掉。
    return !whetherAvailableService(serviceInstanceKey, builtinServices) ? (
        <></>
    ) : (
        serviceInstanceConfig !== null && (
            <div className='bg-content2 rounded-md px-[10px] py-[20px] flex justify-between'>
                <div className='flex'>
                    <div
                        {...drag}
                        className='text-2xl my-auto'
                    >
                        <RxDragHandleHorizontal />
                    </div>

                    <Spacer x={2} />
                    {aiIcon ? (
                        <ServiceIcon id={aiIcon} />
                    ) : (
                        <img
                            src={`${builtinServices[serviceName].info.icon}`}
                            className='h-[24px] w-[24px] my-auto'
                            draggable={false}
                        />
                    )}
                    <Spacer x={2} />
                    <h2 className='my-auto'>
                        {getDisplayInstanceName(serviceInstanceConfig[INSTANCE_NAME_CONFIG_KEY], () =>
                            t(`services.translate.${serviceName}.title`)
                        )}
                    </h2>
                </div>
                <div className='flex'>
                    <Switch
                        size='sm'
                        isSelected={serviceInstanceConfig['enable'] ?? true}
                        onValueChange={(v) => {
                            setServiceInstanceConfig({ ...serviceInstanceConfig, enable: v });
                        }}
                    />
                    <Button
                        isIconOnly
                        size='sm'
                        variant='light'
                        onPress={() => {
                            setCurrentConfigKey(serviceInstanceKey);
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
                            deleteServiceInstance(serviceInstanceKey);
                        }}
                    >
                        <MdDeleteOutline className='text-2xl' />
                    </Button>
                </div>
            </div>
        )
    );
}
