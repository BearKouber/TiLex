import { INSTANCE_NAME_CONFIG_KEY } from '../../../utils/service_instance';
import { Input, Button, Select, SelectItem } from '@nextui-org/react';
import toast from 'react-hot-toast';
import { useTranslation } from 'react-i18next';
import React, { useState } from 'react';

import { useConfig } from '../../../hooks/useConfig';
import { useToastStyle } from '../../../hooks';
import { translate, Language } from './index';

export function Config(props) {
    const { instanceKey, updateServiceList, onClose } = props;
    const { t } = useTranslation();
    const [config, setConfig] = useConfig(
        instanceKey,
        {
            [INSTANCE_NAME_CONFIG_KEY]: t('services.translate.bing.title'),
            type: 'builtin',
            auth_key: '',
            region: '',
            custom_url: 'https://api.cognitive.microsofttranslator.com',
        },
        { sync: false }
    );
    const [isLoading, setIsLoading] = useState(false);
    const [isTesting, setIsTesting] = useState(false);
    const [testResult, setTestResult] = useState(null);

    const toastStyle = useToastStyle();

    const handleTest = async () => {
        setIsTesting(true);
        setTestResult(null);
        const start = Date.now();
        try {
            const res = await translate('Hello world', Language.auto, Language.zh_cn, { config });
            const latency = Date.now() - start;
            const textRes = typeof res === 'string' ? res : JSON.stringify(res);
            const msg = `${t('config.service.test_success', { defaultValue: '连接正常' })}: "Hello world" → "${textRes}" (${latency}ms)`;
            setTestResult({ success: true, message: msg });
            toast.success(msg, { style: toastStyle });
        } catch (err) {
            const latency = Date.now() - start;
            const errMsg = err?.message || err?.toString() || JSON.stringify(err);
            const msg = `${t('config.service.test_failed', { defaultValue: '测试失败' })}: ${errMsg} (${latency}ms)`;
            setTestResult({ success: false, message: msg });
            toast.error(msg, { style: toastStyle });
        } finally {
            setIsTesting(false);
        }
    };

    return (
        config !== null && (
            <form
                onSubmit={async (e) => {
                    e.preventDefault();
                    setIsLoading(true);
                    try {
                        await translate('Hello world', Language.auto, Language.zh_cn, { config });
                        setConfig(config, true);
                        updateServiceList(instanceKey);
                        onClose();
                    } catch (err) {
                        const errMsg = err?.message || err?.toString() || JSON.stringify(err);
                        toast.error(t('config.service.test_failed', { defaultValue: '测试失败' }) + ': ' + errMsg, { style: toastStyle });
                    } finally {
                        setIsLoading(false);
                    }
                }}
            >
                <div className='config-item'>
                    <h3 className='my-auto'>{t('services.instance_name')}</h3>
                    <Input
                        aria-label={t('services.instance_name')}
                        value={config[INSTANCE_NAME_CONFIG_KEY]}
                        variant='bordered'
                        className='w-[50%]'
                        onValueChange={(value) => {
                            setConfig({
                                ...config,
                                [INSTANCE_NAME_CONFIG_KEY]: value,
                            });
                        }}
                    />
                </div>
                <div className='config-item'>
                    <h3 className='my-auto'>{t('services.translate.bing.type', { defaultValue: '翻译模式' })}</h3>
                    <Select
                        aria-label='type'
                        variant='bordered'
                        className='w-[50%]'
                        disallowEmptySelection
                        selectedKeys={[config.type || 'builtin']}
                        onSelectionChange={(keys) => {
                            const key = [...keys][0];
                            if (key) {
                                setConfig({
                                    ...config,
                                    type: key,
                                });
                            }
                        }}
                    >
                        <SelectItem key='builtin'>
                            {t('services.translate.bing.builtin', { defaultValue: '内置免费 (无需配置)' })}
                        </SelectItem>
                        <SelectItem key='api'>
                            {t('services.translate.bing.api', { defaultValue: 'Azure 官方 API' })}
                        </SelectItem>
                    </Select>
                </div>

                {config.type === 'builtin' ? (
                    <div className='text-xs text-default-500 my-2 px-1'>
                        {t('services.translate.bing.builtin_desc', { defaultValue: '基于微软内置接口，开箱即用，无需配置 API Key。' })}
                    </div>
                ) : (
                    <>
                        <div className='config-item'>
                            <h3 className='my-auto'>{t('services.translate.bing.auth_key', { defaultValue: 'Azure 密钥 (Key)' })}</h3>
                            <Input
                                aria-label={t('services.translate.bing.auth_key')}
                                type='password'
                                value={config.auth_key || ''}
                                variant='bordered'
                                className='w-[50%]'
                                onValueChange={(value) => {
                                    setConfig({
                                        ...config,
                                        auth_key: value,
                                    });
                                }}
                            />
                        </div>
                        <div className='config-item'>
                            <h3 className='my-auto'>{t('services.translate.bing.region', { defaultValue: '区域 (Region)' })}</h3>
                            <Input
                                aria-label={t('services.translate.bing.region')}
                                placeholder={t('services.translate.bing.region_placeholder', { defaultValue: '如 eastasia，留空则为全局' })}
                                value={config.region || ''}
                                variant='bordered'
                                className='w-[50%]'
                                onValueChange={(value) => {
                                    setConfig({
                                        ...config,
                                        region: value,
                                    });
                                }}
                            />
                        </div>
                        <div className='config-item'>
                            <h3 className='my-auto'>{t('services.translate.bing.custom_url', { defaultValue: 'Endpoint 地址' })}</h3>
                            <Input
                                aria-label={t('services.translate.bing.custom_url')}
                                value={config.custom_url || ''}
                                variant='bordered'
                                className='w-[50%]'
                                onValueChange={(value) => {
                                    setConfig({
                                        ...config,
                                        custom_url: value,
                                    });
                                }}
                            />
                        </div>
                    </>
                )}

                {testResult && (
                    <div
                        className={`p-3 my-2 rounded-medium text-xs break-all border ${
                            testResult.success
                                ? 'bg-success-50 text-success-700 border-success-200 dark:bg-success-950/40 dark:text-success-300 dark:border-success-800'
                                : 'bg-danger-50 text-danger-700 border-danger-200 dark:bg-danger-950/40 dark:text-danger-300 dark:border-danger-800'
                        }`}
                    >
                        {testResult.message}
                    </div>
                )}

                <div className='flex gap-2 pt-2'>
                    <Button
                        type='button'
                        variant='bordered'
                        isLoading={isTesting}
                        onPress={handleTest}
                        className='flex-1'
                    >
                        {t('config.service.test_connection', { defaultValue: '测试连通性' })}
                    </Button>
                    <Button
                        type='submit'
                        isLoading={isLoading}
                        color='primary'
                        className='flex-1'
                    >
                        {t('common.save')}
                    </Button>
                </div>
            </form>
        )
    );
}

