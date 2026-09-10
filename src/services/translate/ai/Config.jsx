import { Input, Button, Textarea, Select, SelectItem, Spacer } from '@nextui-org/react';
import { Autocomplete, AutocompleteItem, useDisclosure } from '@nextui-org/react';
import { MdRefresh, MdDeleteOutline } from 'react-icons/md';
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

export const defaultRequestArguments = JSON.stringify({
    temperature: 0.1,
    top_p: 0.99,
    frequency_penalty: 0,
    presence_penalty: 0,
});

export function Config(props) {
    const { instanceKey, presetConfig, updateServiceList, onClose } = props;
    const { t } = useTranslation();
    const { isOpen: isIconOpen, onOpen: onIconOpen, onOpenChange: onIconOpenChange } = useDisclosure();
    // 从「添加 AI 服务」的厂商网格进来时，预设就是这个新实例的初值 —— 实例 key
    // 是新的，store 里读不到，useConfig 会原样用这份默认值。
    // ponytail: useConfig 读不到 key 时会把默认值直接落盘，所以点了厂商又取消，
    // config.json 里会留一条不在 translate_service_list 里的孤儿配置。空实例那条
    // 老路径本来就这样，没变差；真要收就得在 useConfig 上开个「先别写」的口子。
    const [aiConfig, setAiConfig] = useConfig(
        instanceKey,
        {
            // 名字留空，让用户自己填；列表里会回落到服务名
            [INSTANCE_NAME_CONFIG_KEY]: '',
            requestPath: '',
            model: '',
            apiKey: '',
            apiFormat: DEFAULT_FORMAT,
            // 流式那条路走的是浏览器 fetch 而不是 tauri http，面板又只有 320px，
            // 批次 8 的 AI 分析还要完整 JSON —— 所以固定关掉，不给开关。
            stream: false,
            promptList: [
                {
                    role: 'system',
                    content:
                        'You are a professional translation engine, please translate the text into a colloquial, professional, elegant and fluent content, without the style of machine translation. You must only translate the text content, never interpret it.',
                },
                { role: 'user', content: `Translate into $to:\n"""\n$text\n"""` },
            ],
            requestArguments: defaultRequestArguments,
            ...(presetConfig ?? {}),
        },
        { sync: false }
    );
    // 兼容旧版本
    if (aiConfig) {
        if (aiConfig.promptList === undefined) {
            setAiConfig({
                ...aiConfig,
                promptList: [
                    {
                        role: 'system',
                        content:
                            'You are a professional translation engine, please translate the text into a colloquial, professional, elegant and fluent content, without the style of machine translation. You must only translate the text content, never interpret it.',
                    },
                    { role: 'user', content: `Translate into $to:\n"""\n$text\n"""` },
                ],
            });
        }
        if (aiConfig.requestArguments === undefined) {
            setAiConfig({
                ...aiConfig,
                requestArguments: defaultRequestArguments,
            });
        }
    }

    const [fetching, setFetching] = useState(false);
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

    // 上次拉到的列表存在 localStorage 里，重开弹窗直接还在。
    // 手填的模型名不一定在列表里，补进去，否则看着像把配置弄丢了。
    const cachedModels = getModels(requestPath, apiFormat);
    const modelOptions = cachedModels.includes(aiConfig?.model)
        ? cachedModels
        : [aiConfig?.model, ...cachedModels].filter(Boolean);

    return (
        aiConfig !== null && (
            <form
                onSubmit={(e) => {
                    e.preventDefault();
                    // 先存先关，测试放后台：测试是真打一次模型，推理模型要好几秒，
                    // 原来整段转圈都在等它。失败照样弹 toast —— Toaster 挂在窗口根部，
                    // 弹窗关了也看得见。代价是连不通的配置也会存下来，列表里点开再改。
                    setAiConfig(aiConfig, true);
                    updateServiceList(instanceKey);
                    onClose();
                    translate('hello', Language.auto, Language.zh_cn, { config: aiConfig }).catch((e) =>
                        toast.error(t('config.service.test_failed') + e.toString(), { style: toastStyle })
                    );
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
                <h3 className='my-auto'>Prompt List</h3>
                <p className='text-[10px] text-default-700'>{t('services.translate.ai.prompt_description')}</p>

                <div className='bg-content2 rounded-[10px] p-3'>
                    {aiConfig.promptList &&
                        aiConfig.promptList.map((prompt, index) => {
                            return (
                                <div className='config-item'>
                                    <Textarea
                                        label={prompt.role}
                                        labelPlacement='outside'
                                        variant='bordered'
                                        value={prompt.content}
                                        placeholder={`Input Some ${prompt.role} Prompt`}
                                        onValueChange={(value) => {
                                            setAiConfig({
                                                ...aiConfig,
                                                promptList: aiConfig.promptList.map((p, i) => {
                                                    if (i === index) {
                                                        if (i === 0) {
                                                            return {
                                                                role: 'system',
                                                                content: value,
                                                            };
                                                        } else {
                                                            return {
                                                                role: index % 2 !== 0 ? 'user' : 'assistant',
                                                                content: value,
                                                            };
                                                        }
                                                    } else {
                                                        return p;
                                                    }
                                                }),
                                            });
                                        }}
                                    />
                                    <Button
                                        isIconOnly
                                        color='danger'
                                        className='my-auto mx-1'
                                        variant='bordered'
                                        onPress={() => {
                                            setAiConfig({
                                                ...aiConfig,
                                                promptList: aiConfig.promptList.filter((_, i) => i !== index),
                                            });
                                        }}
                                    >
                                        <MdDeleteOutline className='text-[18px]' />
                                    </Button>
                                </div>
                            );
                        })}
                    <Button
                        fullWidth
                        variant='bordered'
                        onPress={() => {
                            setAiConfig({
                                ...aiConfig,
                                promptList: [
                                    ...aiConfig.promptList,
                                    {
                                        role:
                                            aiConfig.promptList.length === 0
                                                ? 'system'
                                                : aiConfig.promptList.length % 2 === 0
                                                  ? 'assistant'
                                                  : 'user',
                                        content: '',
                                    },
                                ],
                            });
                        }}
                    >
                        {t('services.translate.ai.add')}
                    </Button>
                </div>
                <br />

                <h3 className='my-auto'>Request Arguments</h3>
                <div className='config-item'>
                    <Textarea
                        label=''
                        labelPlacement='outside'
                        variant='bordered'
                        value={aiConfig['requestArguments']}
                        placeholder={`Input API Request Arguments`}
                        onValueChange={(value) => {
                            setAiConfig({
                                ...aiConfig,
                                requestArguments: value,
                            });
                        }}
                    />
                </div>
                <br />
                <Button
                    type='submit'
                    fullWidth
                    color='primary'
                >
                    {t('common.save')}
                </Button>
            </form>
        )
    );
}
