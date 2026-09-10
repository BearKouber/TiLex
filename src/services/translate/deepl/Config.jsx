import { INSTANCE_NAME_CONFIG_KEY } from '../../../utils/service_instance';
import { Input, Button, Select, SelectItem } from '@nextui-org/react';
import toast from 'react-hot-toast';
import { useTranslation } from 'react-i18next';
import { open } from '@tauri-apps/api/shell';
import React, { useState } from 'react';

import { useConfig } from '../../../hooks/useConfig';
import { useToastStyle } from '../../../hooks';
import { translate } from './index';
import { Language } from './index';

export function Config(props) {
    const { instanceKey, updateServiceList, onClose } = props;
    const { t } = useTranslation();
    const [deeplConfig, setDeeplConfig] = useConfig(
        instanceKey,
        {
            [INSTANCE_NAME_CONFIG_KEY]: t('services.translate.deepl.title'),
            type: 'free',
            authKey: '',
            customUrl: '',
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
            const res = await translate('Hello world', Language.auto, Language.zh_cn, { config: deeplConfig });
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
        deeplConfig !== null && (
            <form
                onSubmit={async (e) => {
                    e.preventDefault();
                    setIsLoading(true);
                    try {
                        await translate('Hello world', Language.auto, Language.zh_cn, { config: deeplConfig });
                        setDeeplConfig(deeplConfig, true);
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
                        value={deeplConfig[INSTANCE_NAME_CONFIG_KEY]}
                        variant='bordered'
                        className='w-[50%]'
                        onValueChange={(value) => {
                            setDeeplConfig({
                                ...deeplConfig,
                                [INSTANCE_NAME_CONFIG_KEY]: value,
                            });
                        }}
                    />
                </div>
                <div className='config-item'>
                    <h3 className='my-auto'>{t('services.translate.deepl.type')}</h3>
                    <Select
                        aria-label='type'
                        variant='bordered'
                        className='w-[50%]'
                        disallowEmptySelection
                        selectedKeys={[deeplConfig.type || 'free']}
                        onSelectionChange={(keys) => {
                            const key = [...keys][0];
                            if (key) {
                                setDeeplConfig({
                                    ...deeplConfig,
                                    type: key,
                                });
                            }
                        }}
                    >
                        <SelectItem key='free'>{t('services.translate.deepl.free')}</SelectItem>
                        <SelectItem key='api'>{t('services.translate.deepl.api')}</SelectItem>
                        <SelectItem key='deeplx'>{t('services.translate.deepl.deeplx')}</SelectItem>
                    </Select>
                </div>
                {deeplConfig.type !== 'free' && (
                    <div className='config-item'>
                        <h3 className='my-auto'>{t('services.help')}</h3>
                        <Button
                            variant='bordered'
                            className='w-[50%]'
                            onPress={() => {
                                const url =
                                    deeplConfig.type === 'api'
                                        ? 'https://developers.deepl.com/docs'
                                        : 'https://github.com/OwO-Network/DeepLX';
                                open(url);
                            }}
                        >
                            {t('services.help')}
                        </Button>
                    </div>
                )}
                {deeplConfig.type === 'api' && (
                    <div className='config-item'>
                        <h3 className='my-auto'>{t('services.translate.deepl.auth_key')}</h3>
                        <Input
                            aria-label={t('services.translate.deepl.auth_key')}
                            type='password'
                            value={deeplConfig['authKey'] || ''}
                            variant='bordered'
                            className='w-[50%]'
                            onValueChange={(value) => {
                                setDeeplConfig({
                                    ...deeplConfig,
                                    authKey: value,
                                });
                            }}
                        />
                    </div>
                )}
                {deeplConfig.type === 'deeplx' && (
                    <div className='config-item'>
                        <h3 className='my-auto'>{t('services.translate.deepl.custom_url')}</h3>
                        <Input
                            aria-label={t('services.translate.deepl.custom_url')}
                            value={deeplConfig.customUrl || ''}
                            variant='bordered'
                            className='w-[50%]'
                            onValueChange={(value) => {
                                setDeeplConfig({
                                    ...deeplConfig,
                                    customUrl: value,
                                });
                            }}
                        />
                    </div>
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
