import {
    Button,
    Card,
    CardBody,
    Checkbox,
    Input,
    Listbox,
    ListboxItem,
    Modal,
    ModalContent,
    ModalFooter,
    ModalHeader,
    Tab,
    Tabs,
} from '@nextui-org/react';
import { MdDeleteOutline, MdDownload, MdVolumeUp } from 'react-icons/md';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import toast from 'react-hot-toast';
import { useTranslation } from 'react-i18next';

import { parseDetail, wordSummary } from '../../../../utils/wordbook_format';
import {
    removeFromWordbook,
    softDeleteWordbookEntries,
    visibleWordbookEntries,
} from '../../../../utils/wordbook_selection';
import { getDB } from '../../../../utils/wordbook';
import { exportMarkdown } from '../../../../utils/wordbook_export';
import { speak } from '../../../../utils/speak';
import { useToastStyle } from '../../../../hooks';
import { listen } from '@tauri-apps/api/event';
import { entryDisplay } from '../../../../utils/saved_entry';
import TranslationResult from '../../../../components/TranslationResult';

// 生词本页面。数据全在 wordbook.db 的 entries 表（结构见 docs/整合方案.md §4.1）。
//
// 一次把没删的记录全读进内存，搜索和筛选都在 JS 里做：几千条量级，
// 每敲一个字重查一次库不划算，也省掉 LIKE 的转义。
// ponytail: 全量载入，条数上万再改成分页 + SQL 过滤。

export default function Wordbook() {
    const [view, setView] = useState({ entries: [], keyword: '', filter: 'all', selectedId: null });
    const { entries, keyword, filter, selectedId } = view;
    const [batchMode, setBatchMode] = useState(false);
    const [checkedIds, setCheckedIds] = useState(new Set());
    const [confirmIds, setConfirmIds] = useState(null);
    const [removing, setRemoving] = useState(false);
    const removalPending = useRef(false);
    const toastStyle = useToastStyle();
    const { t } = useTranslation();
    const loadVersion = useRef(0);
    const reloadNeeded = useRef(false);

    const load = useCallback(async () => {
        reloadNeeded.current = true;
        if (removalPending.current) return;
        const version = ++loadVersion.current;
        try {
            const db = await getDB();
            const rows = await db.select('SELECT * FROM entries WHERE deleted=0 ORDER BY id DESC');
            // Preserve the visible order until the pending deletion chooses its successor.
            if (version === loadVersion.current && !removalPending.current) {
                setView((prev) => ({
                    ...prev,
                    entries: rows.map((r) => ({ ...r, detail: parseDetail(r.detail) })),
                }));
            }
        } finally {
            if (version === loadVersion.current && !removalPending.current) reloadNeeded.current = false;
        }
    }, []);

    useEffect(() => {
        const unlisten = listen('wordbook_changed', () => {
            void load().catch(console.error);
        });
        void unlisten.then(() => load()).catch(console.error);
        return () => {
            void unlisten.then((stop) => stop());
        };
    }, [load]);

    const list = useMemo(() => visibleWordbookEntries({ entries, keyword, filter }), [entries, keyword, filter]);

    // 初次载入和筛选仍默认首项；删除后的顺延由 removeFromWordbook 决定。
    const selected = list.find((e) => e.id === selectedId) ?? list[0] ?? null;
    const display = selected ? entryDisplay(selected) : null;

    const checkedVisibleIds = list.filter((entry) => checkedIds.has(entry.id)).map((entry) => entry.id);
    const allChecked = list.length > 0 && checkedVisibleIds.length === list.length;

    const setSelectedId = (id) => setView((prev) => ({ ...prev, selectedId: id }));
    const changeFilter = (key, value) => {
        setView((prev) => ({ ...prev, [key]: value }));
        setCheckedIds(new Set());
    };
    const finishBatch = () => {
        setBatchMode(false);
        setCheckedIds(new Set());
    };
    const toggleChecked = (id, checked) => {
        setCheckedIds((prev) => {
            const next = new Set(prev);
            if (checked) next.add(id);
            else next.delete(id);
            return next;
        });
    };
    const remove = async (ids, batch = false) => {
        if (removalPending.current || ids.length === 0) return;
        removalPending.current = true;
        setRemoving(true);
        try {
            const db = await getDB();
            await softDeleteWordbookEntries(db, ids);
            // Ignore loads started before the deletion settled; they may contain deleted rows.
            ++loadVersion.current;
            setView((prev) => removeFromWordbook(prev, ids));
            if (batch) {
                finishBatch();
                setConfirmIds(null);
            } else {
                setCheckedIds((prev) => new Set([...prev].filter((id) => !ids.includes(id))));
            }
        } catch (error) {
            toast.error(t('config.wordbook.delete_failed'), { style: toastStyle });
        } finally {
            removalPending.current = false;
            setRemoving(false);
            // Replay skipped refreshes so an unrelated new entry is not lost.
            if (reloadNeeded.current) void load().catch(console.error);
        }
    };

    const doExport = async () => {
        try {
            const path = await exportMarkdown();
            if (path) toast.success(path, { style: toastStyle });
        } catch (e) {
            toast.error(e.toString(), { style: toastStyle });
        }
    };

    return (
        <div className='flex h-full min-h-0 gap-[10px]'>
            <Card
                shadow='none'
                className='w-[280px] shrink-0 h-full border-1 border-default-100'
            >
                <CardBody className='flex flex-col min-h-0 gap-[8px] p-[8px] overflow-hidden'>
                    <div className='flex h-[32px] shrink-0 items-center gap-[4px]'>
                        {batchMode ? (
                            <>
                                <Checkbox
                                    size='sm'
                                    classNames={{ base: 'm-0 max-w-full flex-1 p-0', label: 'text-[12px]' }}
                                    isSelected={allChecked}
                                    isIndeterminate={checkedVisibleIds.length > 0 && !allChecked}
                                    isDisabled={list.length === 0 || removing}
                                    onValueChange={(checked) =>
                                        setCheckedIds(new Set(checked ? list.map((entry) => entry.id) : []))
                                    }
                                >
                                    {t('config.wordbook.select_all_results')}
                                </Checkbox>
                                <Button
                                    size='sm'
                                    variant='light'
                                    className='min-w-0 shrink-0 px-[8px]'
                                    isDisabled={removing}
                                    onPress={finishBatch}
                                >
                                    {t('config.wordbook.done')}
                                </Button>
                            </>
                        ) : (
                            <>
                                <Button
                                    size='sm'
                                    className='min-w-0 flex-1 px-[8px]'
                                    variant='bordered'
                                    startContent={<MdDownload className='shrink-0 text-[16px]' />}
                                    onPress={doExport}
                                >
                                    {t('config.wordbook.export')}
                                </Button>
                                <Button
                                    size='sm'
                                    variant='light'
                                    className='min-w-0 shrink-0 px-[8px]'
                                    isDisabled={removing || list.length === 0}
                                    onPress={() => setBatchMode(true)}
                                >
                                    {t('config.wordbook.multi_select')}
                                </Button>
                            </>
                        )}
                    </div>
                    <Input
                        size='sm'
                        classNames={{ base: 'shrink-0' }}
                        isClearable
                        value={keyword}
                        onValueChange={(value) => changeFilter('keyword', value)}
                        placeholder={t('config.wordbook.search')}
                    />
                    <Tabs
                        size='sm'
                        fullWidth
                        classNames={{ base: 'shrink-0', panel: 'hidden' }}
                        selectedKey={filter}
                        onSelectionChange={(value) => changeFilter('filter', value)}
                    >
                        <Tab
                            key='all'
                            title={t('config.wordbook.all')}
                        />
                        <Tab
                            key='word'
                            title={t('config.wordbook.word')}
                        />
                        <Tab
                            key='sentence'
                            title={t('config.wordbook.sentence')}
                        />
                    </Tabs>
                    {batchMode ? (
                        <div className='flex-1 min-h-0 overflow-y-auto overflow-x-hidden'>
                            {list.length === 0 ? (
                                <div className='p-[8px] text-center text-small text-default-400'>
                                    {t('config.wordbook.empty')}
                                </div>
                            ) : (
                                <ul
                                    aria-label={t('config.wordbook.title')}
                                    className='flex flex-col gap-[4px]'
                                >
                                    {list.map((item) => (
                                        <li
                                            key={item.id}
                                            className={`flex items-center gap-[4px] rounded-small px-[8px] ${selected?.id === item.id ? 'bg-primary/10' : 'hover:bg-default-100'}`}
                                        >
                                            <Checkbox
                                                size='sm'
                                                aria-label={t('config.wordbook.select_entry', { text: item.text })}
                                                classNames={{ base: 'm-0 shrink-0 p-0', wrapper: 'mr-0' }}
                                                isSelected={checkedIds.has(item.id)}
                                                isDisabled={removing}
                                                onValueChange={(checked) => toggleChecked(item.id, checked)}
                                            />
                                            <button
                                                type='button'
                                                className='min-w-0 flex-1 rounded-small px-[4px] py-[6px] text-left outline-none focus-visible:ring-2 focus-visible:ring-primary'
                                                aria-current={selected?.id === item.id ? 'true' : undefined}
                                                onClick={() => setSelectedId(item.id)}
                                            >
                                                <div className='truncate text-[13px]'>{item.text}</div>
                                                <div className='truncate text-[11px] text-default-400'>
                                                    {wordSummary(item.detail) || item.translation}
                                                </div>
                                            </button>
                                        </li>
                                    ))}
                                </ul>
                            )}
                        </div>
                    ) : (
                        <Listbox
                            aria-label={t('config.wordbook.title')}
                            className='flex-1 min-h-0 overflow-y-auto overflow-x-hidden p-0'
                            // 默认的选中态是 bg-default，压在第二行译文上就看不见了。
                            // ⚠️ 光改 data-[selected] 不够：点过之后 item 还带 focus，
                            // NextUI 的 data-[selectable=true]:focus:bg-default 会盖回去
                            // ——鼠标没点时是浅蓝、点完变深灰就是漏了这条。
                            itemClasses={{
                                base: [
                                    'data-[selected=true]:bg-primary/10',
                                    'data-[selectable=true]:focus:bg-primary/10',
                                    'data-[selectable=true]:focus:text-foreground',
                                    'data-[hover=true]:bg-default-100',
                                ].join(' '),
                            }}
                            emptyContent={t('config.wordbook.empty')}
                            selectionMode='single'
                            disallowEmptySelection
                            selectedKeys={selected ? [String(selected.id)] : []}
                            onSelectionChange={(keys) => setSelectedId(Number([...keys][0]))}
                        >
                            {list.map((item) => (
                                <ListboxItem
                                    key={String(item.id)}
                                    textValue={item.text}
                                >
                                    <div className='truncate text-[13px]'>{item.text}</div>
                                    <div className='truncate text-[11px] text-default-400'>
                                        {wordSummary(item.detail) || item.translation}
                                    </div>
                                </ListboxItem>
                            ))}
                        </Listbox>
                    )}
                    {batchMode && (
                        <div className='flex shrink-0 items-center justify-between gap-[8px] border-t-1 border-default-100 pt-[8px]'>
                            <span
                                className='text-[12px] text-default-500'
                                aria-live='polite'
                            >
                                {t('config.wordbook.selected_count', { count: checkedVisibleIds.length })}
                            </span>
                            <Button
                                size='sm'
                                variant='light'
                                color='danger'
                                isDisabled={checkedVisibleIds.length === 0 || removing}
                                onPress={() => setConfirmIds([...checkedVisibleIds])}
                            >
                                {t('config.wordbook.delete')}
                            </Button>
                        </div>
                    )}
                </CardBody>
            </Card>

            <Card
                shadow='none'
                className='grow min-w-0 h-full border-1 border-default-100'
            >
                <CardBody className='p-[20px] overflow-y-auto overflow-x-hidden'>
                    {selected === null ? (
                        <div className='m-auto text-default-400'>{t('config.wordbook.empty')}</div>
                    ) : (
                        <>
                            <div className='flex items-start justify-between gap-[12px]'>
                                <h2 className='min-w-0 text-[18px] select-text whitespace-pre-wrap break-words [overflow-wrap:anywhere]'>
                                    {selected.text}
                                </h2>
                                <div className='flex gap-[4px] shrink-0'>
                                    <Button
                                        isIconOnly
                                        size='sm'
                                        variant='light'
                                        aria-label={t('config.wordbook.speak')}
                                        onPress={() => speak(selected.text)}
                                    >
                                        <MdVolumeUp className='text-[18px]' />
                                    </Button>
                                    <Button
                                        isIconOnly
                                        size='sm'
                                        variant='light'
                                        color='danger'
                                        aria-label={t('config.wordbook.delete')}
                                        isDisabled={removing}
                                        onPress={() => remove([selected.id])}
                                    >
                                        <MdDeleteOutline className='text-[18px]' />
                                    </Button>
                                </div>
                            </div>

                            <div className='mt-[12px]'>
                                <TranslationResult display={display} />
                            </div>
                        </>
                    )}
                </CardBody>
            </Card>
            <Modal
                size='sm'
                isOpen={confirmIds !== null}
                isDismissable={!removing}
                isKeyboardDismissDisabled={removing}
                hideCloseButton
                onOpenChange={(open) => {
                    if (!open && !removalPending.current) setConfirmIds(null);
                }}
            >
                <ModalContent
                    onKeyDownCapture={(event) => {
                        // Disabled modal dismissal must not bubble Escape to the app's close shortcut.
                        if (event.key === 'Escape' && removalPending.current) event.stopPropagation();
                    }}
                >
                    <ModalHeader className='text-[16px]'>
                        {t('config.wordbook.confirm_delete', { count: confirmIds?.length ?? 0 })}
                    </ModalHeader>
                    <ModalFooter>
                        <Button
                            size='sm'
                            variant='light'
                            isDisabled={removing}
                            onPress={() => setConfirmIds(null)}
                        >
                            {t('common.cancel')}
                        </Button>
                        <Button
                            size='sm'
                            color='danger'
                            isLoading={removing}
                            onPress={() => remove(confirmIds ?? [], true)}
                        >
                            {t('config.wordbook.delete_count', { count: confirmIds?.length ?? 0 })}
                        </Button>
                    </ModalFooter>
                </ModalContent>
            </Modal>
        </div>
    );
}
