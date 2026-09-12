import { Input, Button, Textarea, Select, SelectItem, Spacer } from '@nextui-org/react';
import { Autocomplete, AutocompleteItem, useDisclosure } from '@nextui-org/react';
import { MdRefresh } from 'react-icons/md';
import React, { useEffect, useReducer, useState } from 'react';
import { fetch } from '@tauri-apps/api/http';
import toast from 'react-hot-toast';
import { useTranslation } from 'react-i18next';

import { useConfig } from '../../../hooks/useConfig';
import { useToastStyle } from '../../../hooks';
import { translate } from './index';
import { FORMATS, formatOf, DEFAULT_FORMAT } from './protocol';
import { getModels, setModels, subscribe } from './latency';
import { Language } from './index';
import { INSTANCE_NAME_CONFIG_KEY } from '../../../utils/service_instance';
import { ServiceIcon, IconPickerModal } from './ServiceIcon';
import { matchIcon } from './icons';
import ModelBenchmark from './ModelBenchmark';
import { normalizeAiConfig } from './instructions';

export function Config(props) {
    const { instanceKey, presetConfig, updateServiceList, onClose } = props;
    const { t } = useTranslation();
    const { isOpen: isIconOpen, onOpen: onIconOpen, onOpenChange: onIconOpenChange } = useDisclosure();
    // 从「添加 AI 服务」的厂商网格进来时，预设就是这个新实例的初值 —— 实例 key
    // 是新的，store 里读不到，useConfig 会原样用这份默认值。
    const [draftConfig, setAiConfig] = useConfig(
        instanceKey,
        {
            // 名字留空，让用户自己填；列表里会回落到服务名
            [INSTANCE_NAME_CONFIG_KEY]: '',
            requestPath: '',
            model: '',
            apiKey: '',
            apiFormat: DEFAULT_FORMAT,
            ...(presetConfig ?? {}),
        },
        { sync: false }
    );
    // Derive a local normalized view; never set state or persist during rendering.
    const aiConfig = draftConfig === null ? null : normalizeAiConfig(draftConfig);

    const [fetching, setFetching] = useState(false);
    const [testing, setTesting] = useState(false);
    const [saving, setSaving] = useState(false);
    // 测速队列在组件外面跑，它有进展就重画一次
    const [, rerender] = useReducer((n) => n + 1, 0);
    useEffect(() => subscribe(rerender), []);

    const toastStyle = useToastStyle();

    const requestPath = aiConfig?.requestPath ?? '';
    const apiFormat = aiConfig?.apiFormat ?? DEFAULT_FORMAT;
    const fmt = formatOf(apiFormat);
    const iconId = aiConfig?.icon || matchIcon(requestPath, aiConfig?.model);

    // 改地址 / 模型时顺手换脸。用户自己点过图标（iconLocked）就不再动它 ——
    // 否则手选完再改一个字，图标又被抢回去了。
    const patch = (fields) => {
        const next = { ...aiConfig, ...fields };
        if (!next.iconLocked && ('requestPath' in fields || 'model' in fields)) {
            next.icon = matchIcon(next.requestPath, next.model);
        }
        setAiConfig(next);
    };

    // 用户填 base 就够了，真正打出去的是这个地址 —— 在输入框下面标出来，
    // 省得填完了还要猜补全成什么样。
    const resolvedUrl = () => {
        try {
            return requestPath.trim() ? fmt.chatUrl(requestPath, aiConfig?.model || '{model}') : '';
        } catch {
            return '';
        }
    };

    const fetchModels = async () => {
        setFetching(true);
        try {
            const res = await fetch(fmt.modelsUrl(requestPath), {
                method: 'GET',
                headers: { 'Content-Type': 'application/json', ...fmt.headers(aiConfig.apiKey) },
                timeout: 15,
            });
            const list = res.ok ? fmt.models(res.data) : null;
            if (!list || list.length === 0) {
                // 拉不到就保持原样，绝不清空用户已经填好的 model
                throw new Error(res.ok ? 'empty model list' : `HTTP ${res.status}`);
            }
            setModels(requestPath, list, apiFormat);
            toast.success(t('services.translate.ai.fetch_success', { count: list.length }), {
                style: toastStyle,
            });
        } catch (e) {
            toast.error(`${t('services.translate.ai.fetch_failed')} ${e.toString()}`, { style: toastStyle });
        } finally {
            setFetching(false);
        }
    };

    const testConnection = async () => {
        setTesting(true);
        try {
            await translate('hello', Language.auto, Language.zh_cn, { config: aiConfig });
            toast.success(t('config.service.test_success'), { style: toastStyle });
        } catch (error) {
            toast.error(t('config.service.test_failed') + error.toString(), { style: toastStyle });
        } finally {
            setTesting(false);
        }
    };

    // 上次拉到的列表存在 localStorage 里，重开弹窗直接还在。
    // 手填的模型名不一定在列表里，补进去，否则看着像把配置弄丢了。
    const cachedModels = getModels(requestPath, apiFormat);
    const modelOptions = cachedModels.includes(aiConfig?.model)
        ? cachedModels
        : [aiConfig?.model, ...cachedModels].filter(Boolean);

    return (
        aiConfig !== null && (
            <form
                onSubmit={async (e) => {
                    e.preventDefault();
                    setSaving(true);
                    try {
                        await updateServiceList(instanceKey, normalizeAiConfig(aiConfig));
                        onClose();
                    } catch {
                        toast.error(t('config.save_failed'), { style: toastStyle });
                    } finally {
                        setSaving(false);
                    }
                }}
            >
                <IconPickerModal
                    isOpen={isIconOpen}
                    onOpenChange={onIconOpenChange}
                    onPick={(id) => setAiConfig({ ...aiConfig, icon: id, iconLocked: true })}
                />
                <div className='config-item'>
                    <h3 className='my-auto'>{t('services.translate.ai.icon')}</h3>
                    <Button
                        isIconOnly
                        variant='bordered'
                        aria-label={t('services.translate.ai.pick_icon')}
                        title={t('services.translate.ai.pick_icon')}
                        onPress={onIconOpen}
                    >
                        <ServiceIcon id={iconId} />
                    </Button>
                </div>
                <div className='config-item'>
                    <h3 className='my-auto'>{t('services.instance_name')}</h3>
                    <Input
                        aria-label={t('services.instance_name')}
                        value={aiConfig[INSTANCE_NAME_CONFIG_KEY]}
                        placeholder={t('services.translate.ai.title')}
                        variant='bordered'
                        className='w-[50%]'
                        onValueChange={(value) => {
                            setAiConfig({
                                ...aiConfig,
                                [INSTANCE_NAME_CONFIG_KEY]: value,
                            });
                        }}
                    />
                </div>
                <div className='config-item'>
                    <h3 className='my-auto'>{t('services.translate.ai.request_path')}</h3>
                    <Input
                        aria-label={t('services.translate.ai.request_path')}
                        value={aiConfig['requestPath']}
                        placeholder='https://api.example.com/v1'
                        variant='bordered'
                        className='w-[50%]'
                        onValueChange={(value) => patch({ requestPath: value })}
                    />
                </div>
                {resolvedUrl() && (
                    <div className='text-[11px] text-default-400 text-right break-all -mt-[6px] mb-[6px]'>
                        {resolvedUrl()}
                    </div>
                )}
                <div className='config-item'>
                    <h3 className='my-auto'>{t('services.translate.ai.api_key')}</h3>
                    <Input
                        aria-label={t('services.translate.ai.api_key')}
                        type='password'
                        value={aiConfig['apiKey']}
                        variant='bordered'
                        className='w-[50%]'
                        onValueChange={(value) => {
                            setAiConfig({
                                ...aiConfig,
                                apiKey: value,
                            });
                        }}
                    />
                </div>
                <div className='config-item'>
                    <h3 className='my-auto'>{t('services.translate.ai.api_format')}</h3>
                    <Select
                        aria-label='api format'
                        variant='bordered'
                        className='w-[50%]'
                        disallowEmptySelection
                        selectedKeys={[apiFormat]}
                        onSelectionChange={(keys) => {
                            const key = [...keys][0];
                            if (key) {
                                setAiConfig({ ...aiConfig, apiFormat: key });
                            }
                        }}
                    >
                        {Object.entries(FORMATS).map(([key, f]) => (
                            <SelectItem key={key}>{f.label}</SelectItem>
                        ))}
                    </Select>
                </div>
                <div className='config-item'>
                    <h3 className='my-auto'>{t('services.translate.ai.model')}</h3>
                    <div className='flex gap-2 w-[50%]'>
                        <Autocomplete
                            // 有些 OpenAI 兼容端点不实现 /v1/models，拉不出列表时
                            // 必须还能像原来那样直接手打模型名
                            allowsCustomValue
                            aria-label='model'
                            variant='bordered'
                            selectedKey={aiConfig['model']}
                            inputValue={aiConfig['model']}
                            onInputChange={(value) => patch({ model: value })}
                            onSelectionChange={(key) => key && patch({ model: key })}
                        >
                            {modelOptions.map((model) => (
                                <AutocompleteItem
                                    key={model}
                                    textValue={model}
                                >
                                    {model}
                                </AutocompleteItem>
                            ))}
                        </Autocomplete>
                        <Button
                            isIconOnly
                            variant='bordered'
                            className='shrink-0'
                            isLoading={fetching}
                            aria-label={t('services.translate.ai.fetch_models')}
                            title={t('services.translate.ai.fetch_models')}
                            onPress={fetchModels}
                        >
                            <MdRefresh className='text-[20px] text-default-500' />
                        </Button>
                    </div>
                </div>
                <ModelBenchmark
                    requestPath={requestPath}
                    apiKey={aiConfig.apiKey}
                    apiFormat={apiFormat}
                    models={modelOptions}
                    selectedModel={aiConfig.model}
                    onSelectModel={(model) => patch({ model })}
                />
                <Spacer y={2} />
                <Textarea
                    label={t('services.translate.ai.custom_instructions')}
                    labelPlacement='outside'
                    variant='bordered'
                    value={aiConfig.customInstructions}
                    minRows={5}
                    maxRows={12}
                    onValueChange={(value) => patch({ customInstructions: value, legacyReferenceInstructions: '' })}
                />
                {aiConfig.legacyArgumentsInvalid && (
                    <p
                        className='mt-2 text-sm text-warning'
                        role='status'
                    >
                        {t('services.translate.ai.legacy_arguments_invalid')}
                    </p>
                )}
                <Spacer y={3} />
                <Button
                    type='button'
                    variant='bordered'
                    fullWidth
                    isLoading={testing}
                    onPress={testConnection}
                >
                    {t('config.service.test_connection')}
                </Button>
                <Spacer y={2} />
                <Button
                    type='submit'
                    isLoading={saving}
                    fullWidth
                    color='primary'
                >
                    {t('common.save')}
                </Button>
            </form>
        )
    );
}
