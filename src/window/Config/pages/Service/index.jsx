import { Tabs, Tab } from '@nextui-org/react';
import React, { useState } from 'react';
import { useTranslation } from 'react-i18next';
import Translate from './Translate';
import Recognize from './Recognize';
import { osType } from '../../../../utils/env';

export default function Service() {
    const [tab, setTab] = useState('translate');
    const { t } = useTranslation();

    // 顶上一排子 Tab（文本翻译 / 文本识别），一次只渲染一段 —— 原来上下两个大框
    // 堆在一起太挤，两段的行高也对不齐。撑满高度的算式留在这个容器上。
    return (
        <div className={`flex flex-col ${osType === 'Linux' ? 'h-[calc(100vh-140px)]' : 'h-[calc(100vh-120px)]'}`}>
            <Tabs
                size='sm'
                selectedKey={tab}
                onSelectionChange={setTab}
                classNames={{ base: 'shrink-0 w-full justify-center', panel: 'flex-1 min-h-0 flex flex-col pb-0' }}
            >
                <Tab
                    key='translate'
                    title={t('config.service.text_translate')}
                >
                    <Translate />
                </Tab>
                <Tab
                    key='recognize'
                    title={t('config.service.text_recognize')}
                >
                    <Recognize />
                </Tab>
            </Tabs>
        </div>
    );
}
