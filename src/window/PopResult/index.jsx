import { appWindow, currentMonitor, LogicalSize, PhysicalPosition } from '@tauri-apps/api/window';
import React, { useEffect, useRef, useState } from 'react';
import { flushSync } from 'react-dom';
import { writeText } from '@tauri-apps/api/clipboard';
import { speak } from '../../utils/speak';
import PulseLoader from 'react-spinners/PulseLoader';
import { MdCheck, MdChevronRight, MdContentCopy, MdExpandMore, MdStar, MdStarBorder, MdVolumeUp } from 'react-icons/md';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/tauri';
import { createOcrEventGate, ocrErrorMessage } from '../../utils/ocr_request';
import { createBlurGuard } from '../../utils/pop_result_lifecycle';
import { createPopResultSizing } from '../../utils/pop_result_sizing';
import { traceOcr } from '../../utils/ocr_diagnostics';
import { useTranslation } from 'react-i18next';

import { INSTANCE_NAME_CONFIG_KEY, getDisplayInstanceName, getServiceName } from '../../utils/service_instance';
import { cacheKey, getCached, setCached, requestConfigSnapshot } from '../../utils/translate_cache';
import { detectionWithFallback } from '../../utils/detection_fallback';
import * as builtinServices from '../../services/translate';
import { preprocess } from '../../utils/text_preprocess';
import detect from '../../utils/lang_detect';
import { addEntry, updateEntry } from '../../utils/wordbook';
import { createSavedEntry, entryDisplay, resultText as plainText } from '../../utils/saved_entry';
import TranslationResult from '../../components/TranslationResult';
import { store } from '../../utils/store';

// The floating panel the PopButton opens into. Rust positions it, shows it and
// pushes the selection over as `new_text`; the window is never closed, only
// hidden, so all of this state has to be reset on every run.
//
// Which engines appear is the same `translate_service_list` the main window
// uses, minus the instances whose config says `enable: false`.
// 原文行那两个按钮在 copied 里占的位。服务实例 key 不会带 # 号。
const SOURCE_KEY = '#source';

const WIDTH = 320;
// Rust 按这个高度建 WebView 并一直保持（pop_button.rs 的 keep_view_at_max），
// 窗口只是裁剪框，所以变高时下面早就画好了。两边要一起改。
const MAX_HEIGHT = 400;

// 原文那一行的语种徽标。常见的几种写死，其余退回 i18n 语言名的首字
// （中文里「法语」→「法」，英文界面里 'French' → 'F'，都够认）。
const LANG_BADGE = { zh_cn: '中', zh_tw: '繁', en: '英', ja: '日', ko: '韩' };

// Module scope on purpose: the listeners are registered once and would
// otherwise close over a stale render.
let runID = 0;
// Rust 把面板缩成 1px 再显示（pop_button.rs show_result），由这边按新内容撑开。
// pop_anchor 到新内容提交之间 DOM 还是上一次的结果，这时量出来的高度会把窗口
// 撑回旧尺寸、闪一下旧画面，所以这段时间 measure 一律报 0（协调器会丢掉）。
let awaitingText = false;
// 关闭角的保险：面板以光标为基准弹出、或被屏幕边缘挤回来时，光标可能正落在
// 关闭角附近，手还在动，顺势一蹭，刚弹出就闪没。所以指针得先离开面板里第一次
// 出现的位置 ARM_PX 以上，关闭角才生效。
const ARM_PX = 24;
let origin = null;
let armed = false;

export default function PopResult() {
    const [source, setSource] = useState('');
    const [lang, setLang] = useState('');
    const [items, setItems] = useState([]);
    const [saved, setSaved] = useState('');
    const [savedKey, setSavedKey] = useState('');
    const [copied, setCopied] = useState('');
    // '' 正常 / 'loading' 截图识别中 / 其它 = 识别失败的原话
    const [status, setStatus] = useState('');
    // 只记「谁被收起来了」：默认展开，换一段划词就整个清空。
    const [collapsed, setCollapsed] = useState({});
    const { t } = useTranslation();
    const boxRef = useRef();
    const sizingRef = useRef(null);
    const entryRef = useRef(null);
    const blurRef = useRef(null);

    const patch = (id, key, fields) => {
        if (id !== runID) return;
        setItems((prev) => prev.map((it) => (it.key === key ? { ...it, ...fields } : it)));
    };

    const runOne = async (id, item, text, from, to, detected, entry) => {
        const receive = (fields) => {
            entry.patch(item.key, fields);
            patch(id, item.key, fields);
        };
        try {
            const service = builtinServices[getServiceName(item.key)];
            if (!(from in service.Language) || !(to in service.Language)) {
                throw new Error('Language not supported');
            }
            const ck = cacheKey(text, from, to, item.key, item.config, detected);
            const cached = getCached(ck);
            if (cached !== undefined) {
                receive({ result: cached, loading: false });
                return;
            }
            const value = await service.translate(text, service.Language[from], service.Language[to], {
                config: item.config,
                detect: detected,
                setResult: (v) => receive({ result: v }),
            });
            receive({ result: value, loading: false });
            if (plainText(value)) setCached(ck, value);
        } catch (e) {
            receive({ error: e.toString(), loading: false });
        }
    };

    const run = async (raw, requestId) => {
        origin = null;
        armed = false;
        const id = ++runID;
        blurRef.current?.begin(id, requestId);
        traceOcr('focus-request', { runId: id, requestId, textLength: raw.length });
        void appWindow.setFocus().catch(() => traceOcr('focus-request-failed', { runId: id, requestId }));
        entryRef.current = null;
        const text = preprocess(raw);
        if (id !== runID) return;
        // 先把新内容同步提交进 DOM，再量高度，免得量到上一次的结果。
        flushSync(() => {
            setSource(text);
            setLang('');
            setItems([]);
            setCollapsed({});
            setSaved('');
            setSavedKey('');
            setStatus(text ? '' : 'loading');
        });
        awaitingText = false;
        sizingRef.current?.refresh();
        // 空文本 = 截图识别刚开的头，先把面板亮着转圈占位，正文等 OCR 那边认完
        // 再 emit 一次 new_text 进来。
        if (!text) return;
        const entry = createSavedEntry(text, {
            add: addEntry,
            update: updateEntry,
            onStatus: (value) => { if (id === runID) setSaved(value); },
        });
        entryRef.current = entry;

        // Capture this round's languages before awaiting individual instance reads.
        const [configuredList, configuredFrom, configuredTo] = await Promise.all([
            store.get('translate_service_list'),
            store.get('translate_source_language'),
            store.get('translate_target_language'),
        ]);
        const list = configuredList ?? ['google'];
        const from = configuredFrom ?? 'auto';
        const to = configuredTo ?? 'zh_cn';
        const pending = [];
        for (const key of list) {
            const rawConfig = (await store.get(key)) ?? {};
            if (rawConfig['enable'] === false) continue;
            // 服务被删掉后，config.json 里还留着名字。这里不挡的话，每次划词都
            // 会多出一条永远失败的错误行。
            const name = getServiceName(key);
            if (!(name in builtinServices)) continue;
            pending.push({
                key,
                config: requestConfigSnapshot(rawConfig, name),
                label: getDisplayInstanceName(rawConfig[INSTANCE_NAME_CONFIG_KEY], () =>
                    t(`services.translate.${name}.title`)
                ),
                result: '',
                error: '',
                loading: true,
            });
        }
        if (id !== runID && !entry.requested) return;
        if (id === runID) setItems(pending);
        entry.setItems(pending);

        // 原文已经是目标语言时不该走到这里 —— pop_button.rs 的
        // is_native_language 在「显示按钮」那一步就挡掉了，面板不再自己改
        // 目标语言（原来那个退到第二目标语言的逻辑已删）。
        const { detected, badge } = await detectionWithFallback(detect, text, from);
        if (id !== runID && !entry.requested) return;
        // 上面那段逻辑本来就要 detect 一次，徽标是白捡的：不多发一次请求。
        if (id === runID) setLang(badge);
        pending.forEach((item) => void runOne(id, item, text, from, to, detected, entry));
    };

    useEffect(() => {
        const blur = createBlurGuard({
            isFocused: () => appWindow.isFocused(),
            hide: () => appWindow.hide(),
            trace: traceOcr,
        });
        blurRef.current = blur;
        const gate = createOcrEventGate((requestId) => invoke('screenshot_is_current', { requestId }));
        const unlistenSession = listen('screenshot_session', (e) => {
            if (gate.invalidate(e.payload.requestId)) {
                runID++;
                blur.invalidate();
                sizingRef.current?.invalidate();
                traceOcr('session-changed', { requestId: e.payload.requestId, runId: runID });
            }
        });
        const unlistenAnchor = listen('pop_anchor', (e) => {
            awaitingText = true;
            sizingRef.current?.setAnchor(e.payload);
        });
        const unlistenText = listen('new_text', (e) => void gate.accept(e.payload, run));
        // 截图识别失败走这条，不再退回框选覆盖窗。
        const unlistenErr = listen(
            'recognize_error',
            (e) =>
                void gate.accept(e.payload, (message, requestId) => {
                    runID++;
                    entryRef.current = null;
                    const visibleMessage = ocrErrorMessage(message, t('config.recognize.failed'));
                    flushSync(() => {
                        setSource('');
                        setLang('');
                        setItems([]);
                        setSaved('');
                        setStatus(visibleMessage);
                    });
                    awaitingText = false;
                    sizingRef.current?.refresh();
                    traceOcr('error-displayed', { requestId, runId: runID, errorLength: visibleMessage.length });
                })
        );
        // Switching away is the dismiss gesture. The grace period covers the
        // focus handover right after show(), which lands as a blur first.
        //
        // 但「宽限期内的 blur」不能直接丢掉：如果用户就是在这 300ms 里切走的，
        // 那次 blur 是真的，丢了之后不会再有第二次，面板就永远留在屏幕上了
        // —— 这就是「有时候切窗口它不自动关」的原因，偶发是因为要手快。
        // 改成延到宽限期结束再查一次真实焦点状态：交接抖动那次查出来仍有焦点，
        // 真切走那次查出来没焦点，两种都判对。
        // Global listen also receives Config/Screenshot focus changes. Those
        // can enqueue a hide before this result's new_text has even arrived.
        const unlistenBlur = appWindow.listen('tauri://blur', () => blur.blur());
        const unlistenFocus = appWindow.listen('tauri://focus', () => blur.focus());
        return () => {
            gate.invalidate();
            blur.invalidate();
            blurRef.current = null;
            runID++;
            unlistenSession.then((f) => f());
            unlistenAnchor.then((f) => f());
            unlistenText.then((f) => f());
            unlistenErr.then((f) => f());
            unlistenBlur.then((f) => f());
            unlistenFocus.then((f) => f());
        };
    }, []);

    // The window is sized to whatever the content turns out to be, so a two
    // word translation gets a two word panel. Rust only picks the position.
    useEffect(() => {
        const sizing = createPopResultSizing({
            width: WIDTH,
            measure: () => (awaitingText ? 0 : (boxRef.current?.offsetHeight ?? 0)),
            setSize: (width, height) => appWindow.setSize(new LogicalSize(width, height)),
            currentMonitor,
            outerPosition: () => appWindow.outerPosition(),
            setPosition: (x, y) => appWindow.setPosition(new PhysicalPosition(x, y)),
            // 窗口每次改完尺寸都来一下。DWM 要等窗口里有新画面提交才把新尺寸送上屏，
            // 而 WebView 固定按最大尺寸渲染、改窗口尺寸时并不重画，屏幕就一直停在
            // 旧尺寸（拦腰截断），直到别的窗口（比如鼠标底下的网页）刷新了一帧。
            // RedrawWindow 管不到 Chromium 的合成，只能让页面整块变一下逼它提交两帧。
            repaint: () => {
                const box = boxRef.current;
                if (!box) return;
                box.style.opacity = '0.99';
                setTimeout(() => {
                    box.style.opacity = '';
                }, 100);
            },
        });
        sizingRef.current = sizing;
        // 首次 observe 自带一次回调，挂载时的尺寸也走这里。
        const observer = new ResizeObserver(sizing.request);
        observer.observe(boxRef.current);
        return () => {
            observer.disconnect();
            sizing.dispose();
            sizingRef.current = null;
        };
    }, []);

    // Save immediately; this selection's remaining results update the same row.
    const collect = async (key = null) => {
        if (!source || entryRef.current?.text !== source) return;
        if (key) setSavedKey(key);
        await entryRef.current.save(key);
    };
    // 每个服务一个复制：原来底部那个「全部拼起来」的按钮，从外观上根本看不出
    // 复制的是哪一条。反馈沿用 saved 那一套，两个绿勾长得一样。
    // key 只是用来认「哪个按钮该显示绿勾」，原文行借 SOURCE_KEY 占一个位。
    const copyText = (key, text) => {
        void writeText(text);
        setCopied(key);
        setTimeout(() => setCopied((c) => (c === key ? '' : c)), 1500);
    };
    const toggle = (key) => setCollapsed((prev) => ({ ...prev, [key]: !prev[key] }));

    return (
        <div
            ref={boxRef}
            className='relative w-screen rounded-[8px] bg-content1 border-1 border-default-200 overflow-hidden pb-[4px]'
            onMouseMove={(e) => {
                if (armed) return;
                if (!origin) origin = [e.clientX, e.clientY];
                else armed = Math.hypot(e.clientX - origin[0], e.clientY - origin[1]) > ARM_PX;
            }}
        >
            {/* 左上角红三角：悬停或点击即关。弹出位置是用户选的，光标可能一出来就
                在它旁边（比如向右下展开时正好压在左上角），所以要等 armed。 */}
            <div
                className='absolute top-0 left-0 z-30 cursor-pointer w-[10px] h-[10px]'
                title={t('common.close', { defaultValue: '关闭' })}
                onClick={() => armed && blurRef.current?.dismiss('corner-click')}
                onMouseEnter={() => armed && blurRef.current?.dismiss('corner-hover')}
            >
                <svg
                    className='w-full h-full text-danger hover:text-danger-600 transition-colors'
                    viewBox='0 0 10 10'
                    fill='currentColor'
                >
                    <polygon points='0,0 10,0 0,10' />
                </svg>
            </div>
            <div
                // overflow-x-hidden 是必须的：CSS 里一轴设成非 visible 之后，
                // 另一轴的 visible 会自动变成 auto，所以光写 overflow-y-auto
                // 等于两个方向都能滚，长单词或宽元素就会拖出一条横向滚动条。
                // 原文行与服务行在同一个滚动容器中，共享滚动条，保证无论长短文本右侧均严格对齐；
                // 原文行与服务行的操作按钮统一靠右对齐（ml-auto）。
                className='overflow-y-auto overflow-x-hidden pb-[4px]'
                style={{ maxHeight: MAX_HEIGHT - 6 }}
            >
                {/* 原文行：语种徽标 + 原文，右侧悬停显现朗读 / 复制。
                    按钮不能落进拖拽区 —— data-tauri-drag-region 会把点击整个吞掉，
                    所以拖拽区收到文字那一段上，不再罩住整行。
                    min-w-0：flex item 默认不肯收缩到内容宽度以下，不加这条 truncate
                    就不生效，长原文会把两个按钮挤出面板。 */}
                {source && (
                    <div className='group/source sticky top-0 z-20 bg-content1 px-[8px] h-[24px] flex items-center gap-[2px] text-[11px] text-default-400'>
                        {/* select-none：面板正好开在光标底下，点划词按钮那一下的 mouseup
                            落进这一行，浏览器当成一次拖选，原文一出来就是蓝的。
                            原文要拿走用右边那个复制按钮，不靠手选。
                            译文那边不加 —— 那里是要能划着选的。 */}
                        <span
                            className='truncate min-w-0 select-none'
                            data-tauri-drag-region='true'
                        >
                            {lang && (
                                <span className='text-primary/60 mr-[4px]'>
                                    {LANG_BADGE[lang] ?? t(`languages.${lang}`).slice(0, 1)}
                                </span>
                            )}
                            {source}
                        </span>
                        <div className='ml-auto flex items-center gap-[4px]'>
                            <button
                                className='shrink-0 opacity-0 group-hover/source:opacity-100 hover:text-default-600 transition-opacity'
                                aria-label={t('config.wordbook.speak')}
                                title={t('config.wordbook.speak')}
                                onClick={() => speak(source)}
                            >
                                <MdVolumeUp className='text-[12px]' />
                            </button>
                            <button
                                className={`shrink-0 transition-opacity ${
                                    copied === SOURCE_KEY
                                        ? 'opacity-100 text-success'
                                        : 'opacity-0 group-hover/source:opacity-100 hover:text-default-600'
                                }`}
                                aria-label={t('recognize.copy_text')}
                                title={t('recognize.copy_text')}
                                onClick={() => copyText(SOURCE_KEY, source)}
                            >
                                {copied === SOURCE_KEY ? (
                                    <MdCheck className='text-[12px]' />
                                ) : (
                                    <MdContentCopy className='text-[12px]' />
                                )}
                            </button>
                        </div>
                    </div>
                )}
                {status &&
                    (status === 'loading' ? (
                        <div className='px-[8px] py-[4px] flex items-center gap-[6px] text-[12px] text-default-400'>
                            <PulseLoader
                                size={4}
                                color='#a1a1aa'
                            />
                            {t('recognize.recognizing')}
                        </div>
                    ) : (
                        <div className='px-[8px] py-[4px] text-[12px] text-danger break-words'>{status}</div>
                    ))}
                {items.map((it) => (
                    <div
                        key={it.key}
                        className='py-[2px]'
                    >
                        {/* 表头行：折叠开关 + 服务名 + 仅鼠标移到表头行时显现控件。
                            sticky：向下滚动时吸附在原文行正下方。 */}
                        <div
                            className='group/service sticky z-10 bg-content1 px-[8px] h-[20px] flex items-center gap-[2px] text-[10px] text-default-400 cursor-pointer select-none'
                            style={{ top: source ? '24px' : '0px' }}
                            onClick={() => toggle(it.key)}
                        >
                            {collapsed[it.key] ? <MdChevronRight /> : <MdExpandMore />}
                            <span className='truncate'>{it.label}</span>
                            {plainText(it.result) && (
                                <div className='ml-auto flex items-center gap-[4px]'>
                                    {/* 朗读：悬停显现 */}
                                    <button
                                        className='shrink-0 opacity-0 group-hover/service:opacity-100 hover:text-default-600 transition-opacity'
                                        aria-label={t('config.wordbook.speak')}
                                        title={t('config.wordbook.speak')}
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            speak(plainText(it.result));
                                        }}
                                    >
                                        <MdVolumeUp className='text-[12px]' />
                                    </button>
                                    {/* 复制：悬停显现，复制成功时为绿色对勾 */}
                                    <button
                                        className={`shrink-0 transition-opacity ${
                                            copied === it.key
                                                ? 'opacity-100 text-success'
                                                : 'opacity-0 group-hover/service:opacity-100 hover:text-default-600'
                                        }`}
                                        aria-label={t('recognize.copy_text')}
                                        title={t('recognize.copy_text')}
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            copyText(it.key, plainText(it.result));
                                        }}
                                    >
                                        {copied === it.key ? (
                                            <MdCheck className='text-[12px]' />
                                        ) : (
                                            <MdContentCopy className='text-[12px]' />
                                        )}
                                    </button>
                                    {/* 收藏：未收藏时悬停显现，已收藏时常驻高亮金色星星 */}
                                    <button
                                        className={`shrink-0 transition-opacity ${
                                            savedKey === it.key
                                                ? 'opacity-100 text-warning'
                                                : 'opacity-0 group-hover/service:opacity-100 hover:text-default-600'
                                        }`}
                                        aria-label={t('config.wordbook.title')}
                                        title={t('config.wordbook.title')}
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            void collect(it.key);
                                        }}
                                    >
                                        {savedKey === it.key ? (
                                            <MdStar className='text-[13px]' />
                                        ) : (
                                            <MdStarBorder className='text-[13px]' />
                                        )}
                                    </button>
                                </div>
                            )}
                        </div>
                        <div className='px-[8px]'>
                            {collapsed[it.key] ? null : it.error ? (
                                <div className='text-[12px] text-danger break-words'>{it.error}</div>
                            ) : it.loading && !it.result ? (
                                <PulseLoader
                                    size={4}
                                    color='#a1a1aa'
                                />
                            ) : (
                                <TranslationResult
                                    compact
                                    display={entryDisplay({ detail: it.result, translation: plainText(it.result) })}
                                />
                            )}
                        </div>
                    </div>
                ))}
            </div>
        </div>
    );
}
