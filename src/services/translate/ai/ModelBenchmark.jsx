import React, { useState, useEffect, useReducer } from 'react';
import { Button, Chip, Spinner, Tooltip } from '@nextui-org/react';
import { motion, AnimatePresence } from 'framer-motion';
import { MdBolt, MdExpandMore, MdCheck, MdClose, MdPlayArrow, MdStop } from 'react-icons/md';
import { useTranslation } from 'react-i18next';
import { getAllLatencies, getBenchmarkProgress, measure, stopBenchmark, subscribe } from './latency';

export default function ModelBenchmark(props) {
    const { requestPath, apiKey, apiFormat, models = [], selectedModel, onSelectModel } = props;
    const { t } = useTranslation();
    const [isExpanded, setIsExpanded] = useState(false);

    // 订阅测速事件通知，随时驱动重排与动画
    const [, rerender] = useReducer((n) => n + 1, 0);
    useEffect(() => subscribe(rerender), []);

    const allModels = Array.from(new Set(models.filter(Boolean)));
    const latencies = getAllLatencies(requestPath, apiFormat);
    const progress = getBenchmarkProgress(requestPath, apiFormat);

    // 实时计算有序模型列表
    const sortedList = allModels
        .map((m) => {
            const val = latencies[m];
            const isTesting = progress.currentModel === m || val === 'testing';
            return {
                model: m,
                latency: val,
                isTesting,
            };
        })
        .sort((a, b) => {
            const aIsNum = typeof a.latency === 'number';
            const bIsNum = typeof b.latency === 'number';

            // 1. 已测出数值按速度从快到慢升序排
            if (aIsNum && bIsNum) return a.latency - b.latency;
            if (aIsNum) return -1;
            if (bIsNum) return 1;

            // 2. 正在测试中的排在已测数值之后
            if (a.isTesting && !b.isTesting) return -1;
            if (!a.isTesting && b.isTesting) return 1;

            // 3. 失败的排到最底下
            if (a.latency === 'failed' && b.latency !== 'failed') return 1;
            if (b.latency === 'failed' && a.latency !== 'failed') return -1;

            return 0;
        });

    const testedCount = allModels.filter((m) => typeof latencies[m] === 'number').length;
    const failedCount = allModels.filter((m) => latencies[m] === 'failed').length;

    const handleStartBenchmark = (force = false) => {
        if (!requestPath) return;
        measure(requestPath, apiKey, allModels, apiFormat, force);
    };

    const handleStopBenchmark = () => {
        stopBenchmark(requestPath, apiFormat);
    };

    const getBadgeStyle = (latency, isTesting) => {
        if (isTesting) {
            return { color: 'text-primary', label: t('services.translate.ai.testing') };
        }
        if (latency === 'failed') {
            return { color: 'text-danger font-semibold', label: t('services.translate.ai.failed') };
        }
        if (typeof latency === 'number') {
            if (latency < 350) return { color: 'text-emerald-500 font-bold', label: `${latency}ms` };
            if (latency < 800) return { color: 'text-sky-500 font-semibold', label: `${latency}ms` };
            if (latency < 1500) return { color: 'text-amber-500 font-medium', label: `${latency}ms` };
            return { color: 'text-orange-500 font-medium', label: `${latency}ms` };
        }
        return { color: 'text-default-400', label: t('services.translate.ai.untested') };
    };

    const getRankDisplay = (index, item) => {
        if (item.isTesting)
            return (
                <Spinner
                    size='sm'
                    className='h-3.5 w-3.5'
                />
            );
        if (item.latency === 'failed') return <MdClose className='text-danger text-sm' />;
        if (typeof item.latency === 'number') {
            return <span className='text-default-500 font-mono font-semibold text-xs'>#{index + 1}</span>;
        }
        return <span className='text-default-300 font-mono text-xs'>-</span>;
    };

    return (
        <div className='w-full mt-2 border border-default-200 rounded-xl overflow-hidden bg-content1/50'>
            {/* 顶栏可点击折叠/展开 */}
            <div
                className='flex items-center justify-between px-3 py-2.5 cursor-pointer select-none hover:bg-default-100/60 transition-colors'
                onClick={() => setIsExpanded(!isExpanded)}
            >
                <div className='flex items-center gap-2'>
                    <MdBolt className='text-warning text-lg' />
                    <span className='text-xs font-semibold text-foreground'>
                        {t('services.translate.ai.benchmark_title')}
                    </span>
                    {progress.isRunning ? (
                        <span className='text-[11px] text-primary flex items-center gap-1.5 font-mono ml-1'>
                            <Spinner
                                size='sm'
                                className='h-3.5 w-3.5'
                            />
                            {progress.currentModel || t('services.translate.ai.testing')}
                        </span>
                    ) : testedCount > 0 ? (
                        <span className='text-[11px] text-default-400 ml-1'>
                            (
                            {t('services.translate.ai.benchmark_summary', {
                                tested: testedCount,
                                total: allModels.length,
                            })}
                            {failedCount > 0 &&
                                t('services.translate.ai.benchmark_failed_suffix', { count: failedCount })}
                            )
                        </span>
                    ) : null}
                </div>
                <div className='flex items-center gap-2 text-default-400'>
                    <span className='text-[11px]'>{isExpanded ? t('common.collapse') : t('common.expand')}</span>
                    <MdExpandMore
                        className={`text-base transition-transform duration-200 ${isExpanded ? 'rotate-180' : ''}`}
                    />
                </div>
            </div>

            {/* 展开后的测速详情与动态重排序列表 */}
            {isExpanded && (
                <div className='p-3 border-t border-default-200/60 bg-content2/40'>
                    {/* 操作工具条 */}
                    <div className='flex items-center justify-between gap-2 mb-2.5 pb-2 border-b border-default-200/40'>
                        <div className='flex items-center gap-2'>
                            {progress.isRunning ? (
                                <Button
                                    size='sm'
                                    color='danger'
                                    variant='flat'
                                    className='h-7 text-xs px-2.5'
                                    startContent={<MdStop className='text-sm' />}
                                    onPress={handleStopBenchmark}
                                >
                                    {t('services.translate.ai.stop_benchmark')}
                                </Button>
                            ) : (
                                <Button
                                    size='sm'
                                    color='primary'
                                    variant='flat'
                                    className='h-7 text-xs px-2.5'
                                    isDisabled={allModels.length === 0}
                                    startContent={<MdPlayArrow className='text-sm' />}
                                    onPress={() => handleStartBenchmark(testedCount > 0)}
                                >
                                    {testedCount > 0
                                        ? t('services.translate.ai.retest_benchmark')
                                        : t('services.translate.ai.start_benchmark')}
                                </Button>
                            )}
                            <span className='text-[11px] text-default-400 hidden sm:inline'>
                                {t('services.translate.ai.benchmark_hint')}
                            </span>
                        </div>
                        {progress.isRunning && (
                            <span className='text-[11px] text-primary font-mono shrink-0'>
                                {t('services.translate.ai.benchmark_remaining', { count: progress.remaining })}
                            </span>
                        )}
                    </div>

                    {/* 模型列表 */}
                    {allModels.length === 0 ? (
                        <div className='text-center py-6 text-xs text-default-400'>
                            {t('services.translate.ai.no_models_hint')}
                        </div>
                    ) : (
                        <div className='flex flex-col gap-1.5 max-h-[240px] overflow-y-auto pr-1'>
                            <AnimatePresence initial={false}>
                                {sortedList.map((item, index) => {
                                    const badge = getBadgeStyle(item.latency, item.isTesting);
                                    const isSelected = selectedModel === item.model;

                                    return (
                                        <motion.div
                                            key={item.model}
                                            layout
                                            initial={{ opacity: 0, y: 10 }}
                                            animate={{ opacity: 1, y: 0 }}
                                            exit={{ opacity: 0, scale: 0.96 }}
                                            transition={{
                                                type: 'spring',
                                                stiffness: 380,
                                                damping: 30,
                                            }}
                                            className={`rounded-lg px-2.5 py-1.5 flex items-center justify-between border transition-colors ${
                                                isSelected
                                                    ? 'border-primary/50 bg-primary/10 shadow-sm'
                                                    : 'border-default-200/60 bg-content1 hover:bg-default-100/60'
                                            }`}
                                        >
                                            {/* 左侧：排名与模型名称 */}
                                            <div className='flex items-center gap-2 min-w-0 flex-1 mr-2'>
                                                <div className='w-9 shrink-0 flex items-center justify-center text-xs'>
                                                    {getRankDisplay(index, item)}
                                                </div>
                                                <Tooltip
                                                    content={item.model}
                                                    delay={600}
                                                >
                                                    <span className='font-mono text-xs text-foreground truncate cursor-default select-all'>
                                                        {item.model}
                                                    </span>
                                                </Tooltip>
                                                {isSelected && (
                                                    <Chip
                                                        size='sm'
                                                        variant='flat'
                                                        color='primary'
                                                        className='h-4 px-1 text-[9px] shrink-0'
                                                    >
                                                        {t('services.translate.ai.applied_model')}
                                                    </Chip>
                                                )}
                                            </div>

                                            {/* 右侧：延迟标签与选用按钮 */}
                                            <div className='flex items-center gap-2 shrink-0'>
                                                <span
                                                    className={`text-xs font-mono min-w-[56px] text-right ${badge.color}`}
                                                >
                                                    {badge.label}
                                                </span>
                                                <Button
                                                    size='sm'
                                                    variant={isSelected ? 'solid' : 'bordered'}
                                                    color={isSelected ? 'success' : 'primary'}
                                                    className={`h-6 min-w-[50px] text-xs px-2 ${
                                                        isSelected ? 'cursor-default' : ''
                                                    }`}
                                                    isDisabled={item.latency === 'failed'}
                                                    onPress={() => {
                                                        if (!isSelected && onSelectModel) {
                                                            onSelectModel(item.model);
                                                        }
                                                    }}
                                                >
                                                    {isSelected ? (
                                                        <span className='flex items-center gap-0.5'>
                                                            <MdCheck className='text-xs' />
                                                            {t('services.translate.ai.applied_model')}
                                                        </span>
                                                    ) : (
                                                        t('services.translate.ai.apply_model')
                                                    )}
                                                </Button>
                                            </div>
                                        </motion.div>
                                    );
                                })}
                            </AnimatePresence>
                        </div>
                    )}
                </div>
            )}
        </div>
    );
}
