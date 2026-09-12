import { Button, Card, CardBody, Input, Listbox, ListboxItem, Tab, Tabs } from '@nextui-org/react';
import { MdDeleteOutline, MdDownload, MdVolumeUp } from 'react-icons/md';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import toast from 'react-hot-toast';
import { useTranslation } from 'react-i18next';

import { matchEntry, parseDetail, wordSummary } from '../../../../utils/wordbook_format';
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
    const [entries, setEntries] = useState([]);
    const [keyword, setKeyword] = useState('');
    const [filter, setFilter] = useState('all');
    const [selectedId, setSelectedId] = useState(null);
    const toastStyle = useToastStyle();
    const { t } = useTranslation();
    const loadVersion = useRef(0);

    const load = useCallback(async () => {
        const version = ++loadVersion.current;
        const db = await getDB();
        const rows = await db.select('SELECT * FROM entries WHERE deleted=0 ORDER BY id DESC');
        if (version === loadVersion.current) {
            setEntries(rows.map((r) => ({ ...r, detail: parseDetail(r.detail) })));
        }
    }, []);

    useEffect(() => {
        const unlisten = listen('wordbook_changed', () => { void load().catch(console.error); });
        void unlisten.then(() => load()).catch(console.error);
        return () => { void unlisten.then((stop) => stop()); };
    }, [load]);

    const list = useMemo(() => {
        const kw = keyword.trim().toLowerCase();
        return entries.filter((e) => matchEntry(e, filter, kw));
    }, [entries, keyword, filter]);

    // 选中的条目被筛掉或被删掉时，落到当前列表的第一条上。
    const selected = list.find((e) => e.id === selectedId) ?? list[0] ?? null;
    const display = selected ? entryDisplay(selected) : null;

    const remove = async (id) => {
        const db = await getDB();
        await db.execute('UPDATE entries SET deleted=1 WHERE id=$1', [id]);
        setEntries((prev) => prev.filter((e) => e.id !== id));
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
                    <Button
                        size='sm'
                        className='shrink-0'
                        variant='bordered'
                        startContent={<MdDownload className='text-[16px]' />}
                        onPress={doExport}
                    >
                        {t('config.wordbook.export')}
                    </Button>
                    <Input
                        size='sm'
                        classNames={{ base: 'shrink-0' }}
                        isClearable
                        value={keyword}
                        onValueChange={setKeyword}
                        placeholder={t('config.wordbook.search')}
                    />
                    <Tabs
                        size='sm'
                        fullWidth
                        classNames={{ base: 'shrink-0', panel: 'hidden' }}
                        selectedKey={filter}
                        onSelectionChange={setFilter}
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
                    <Listbox
                        aria-label='wordbook entries'
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
                                        onPress={() => remove(selected.id)}
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
        </div>
    );
}
