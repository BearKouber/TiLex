import { appWindow, currentMonitor, LogicalSize, PhysicalPosition } from '@tauri-apps/api/window';
import React, { useEffect, useRef, useState } from 'react';
import { writeText } from '@tauri-apps/api/clipboard';
import { speak } from '../../utils/speak';
import PulseLoader from 'react-spinners/PulseLoader';
import { MdCheck, MdChevronRight, MdContentCopy, MdExpandMore, MdStarBorder, MdVolumeUp } from 'react-icons/md';
import { listen } from '@tauri-apps/api/event';
import { useTranslation } from 'react-i18next';

import { INSTANCE_NAME_CONFIG_KEY, getDisplayInstanceName, getServiceName } from '../../utils/service_instance';
import { cacheKey, getCached, setCached } from '../../utils/translate_cache';
import * as builtinServices from '../../services/translate';
import { preprocess } from '../../utils/text_preprocess';
import detect from '../../utils/lang_detect';
import { addEntry } from '../../utils/wordbook';
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
const MAX_HEIGHT = 400;

// 原文那一行的语种徽标。常见的几种写死，其余退回 i18n 语言名的首字
// （中文里「法语」→「法」，英文界面里 'French' → 'F'，都够认）。
const LANG_BADGE = { zh_cn: '中', zh_tw: '繁', en: '英', ja: '日', ko: '韩' };

// Module scope on purpose: the listeners are registered once and would
// otherwise close over a stale render.
let runID = 0;
let shownAt = 0;
// Rust 摆放时面板在基准上方，就是面板底边的物理 y（内容变高时钉住它往上长）；
// 在下方是 null（顶边不动往下长）。每次弹出由 pop_anchor 事件先于 new_text 送来。
let pinBottom = null;
// 关闭角的保险：面板以光标为基准弹出、或被屏幕边缘挤回来时，光标可能正落在
// 关闭角附近，手还在动，顺势一蹭，刚弹出就闪没。所以指针得先离开面板里第一次
// 出现的位置 ARM_PX 以上，关闭角才生效。
const ARM_PX = 24;
let origin = null;
let armed = false;

// Several services answer a single word with a dictionary entry rather than a
// sentence - google does it for any word it knows. Results are kept in whatever
// shape they arrive in; this is only for copying and for the length check.
const plainText = (v) =>
    typeof v === 'string'
        ? v.trim()
        : [
              ...(v?.pronunciations ?? []).map((p) => p.symbol).filter(Boolean),
              ...(v?.explanations ?? []).map((e) => `${e.trait ?? ''} ${(e.explains ?? []).join(', ')}`.trim()),
              ...(v?.associations ?? []),
          ].join('\n');

// Compact form of the full window's dictionary view: pronunciation and
// meanings only. Example sentences are what the arrow button is for - they do
// not fit in a panel this size.
function DictView({ data }) {
    const symbols = (data.pronunciations ?? []).map((p) => p.symbol).filter(Boolean);
    return (
        <div className='select-text'>
            {symbols.length > 0 && <div className='text-[11px] text-default-500'>{symbols.join('  ')}</div>}
            {(data.explanations ?? []).map((item, index) => (
                <div
                    key={index}
                    className='text-[13px]'
                >
                    {item.trait && <span className='text-[10px] text-default-400 mr-[6px]'>{item.trait}</span>}
                    {(item.explains ?? []).join(', ')}
                </div>
            ))}
            {(data.associations ?? []).length > 0 && (
                <div className='text-[11px] text-default-500'>{data.associations.join(', ')}</div>
            )}
        </div>
    );
}

export default function PopResult() {
    const [source, setSource] = useState('');
    const [lang, setLang] = useState('');
    const [items, setItems] = useState([]);
    const [saved, setSaved] = useState('');
    const [copied, setCopied] = useState('');
    // '' 正常 / 'loading' 截图识别中 / 其它 = 识别失败的原话
    const [status, setStatus] = useState('');
    // 只记「谁被收起来了」：默认展开，换一段划词就整个清空。
    const [collapsed, setCollapsed] = useState({});
    const { t } = useTranslation();
    const boxRef = useRef();
    const heightRef = useRef(0);

    const patch = (id, key, fields) => {
        if (id !== runID) return;
        setItems((prev) => prev.map((it) => (it.key === key ? { ...it, ...fields } : it)));
    };

    const runOne = async (id, item, text, from, to, detected) => {
        try {
            const service = builtinServices[getServiceName(item.key)];
            if (!(from in service.Language) || !(to in service.Language)) {
                throw new Error('Language not supported');
            }
            const ck = cacheKey(text, from, to, item.key);
            const cached = getCached(ck);
            if (cached !== undefined) {
                patch(id, item.key, { result: cached, loading: false });
                return;
            }
            const value = await service.translate(text, service.Language[from], service.Language[to], {
                config: item.config,
                detect: detected,
                setResult: (v) => patch(id, item.key, { result: v }),
            });
            patch(id, item.key, { result: value, loading: false });
            if (plainText(value)) setCached(ck, value);
        } catch (e) {
            patch(id, item.key, { error: e.toString(), loading: false });
        }
    };

    const run = async (raw) => {
        shownAt = Date.now();
        origin = null;
        armed = false;
        void appWindow.setFocus();
        const id = ++runID;
        const text = preprocess(raw, {
            deleteNewline: (await store.get('translate_delete_newline')) ?? false,
            codeSplit: (await store.get('translate_code_split')) ?? false,
        }).trim();
        if (id !== runID) return;
        setSource(text);
        setLang('');
        setItems([]);
        setCollapsed({});
        setSaved('');
        setStatus(text ? '' : 'loading');
        // 空文本 = 截图识别刚开的头，先把面板亮着转圈占位，正文等 OCR 那边认完
        // 再 emit 一次 new_text 进来。
        if (!text) return;

        const list = (await store.get('translate_service_list')) ?? ['google'];
        const pending = [];
        for (const key of list) {
            const config = (await store.get(key)) ?? {};
            if (config['enable'] === false) continue;
            // 服务被删掉后，config.json 里还留着名字。这里不挡的话，每次划词都
            // 会多出一条永远失败的错误行。
            if (!(getServiceName(key) in builtinServices)) continue;
            pending.push({
                key,
                config,
                label: getDisplayInstanceName(config[INSTANCE_NAME_CONFIG_KEY], () =>
                    t(`services.translate.${getServiceName(key)}.title`)
                ),
                result: '',
                error: '',
                loading: true,
            });
        }
        if (id !== runID) return;
        setItems(pending);

        const from = (await store.get('translate_source_language')) ?? 'auto';
        const to = (await store.get('translate_target_language')) ?? 'zh_cn';
        // 原文已经是目标语言时不该走到这里 —— pop_button.rs 的
        // is_native_language 在「显示按钮」那一步就挡掉了，面板不再自己改
        // 目标语言（原来那个退到第二目标语言的逻辑已删）。
        const detected = await detect(text);
        if (id !== runID) return;
        // 上面那段逻辑本来就要 detect 一次，徽标是白捡的：不多发一次请求。
        setLang(detected);
        pending.forEach((item) => void runOne(id, item, text, from, to, detected));
    };

    useEffect(() => {
        const unlistenAnchor = listen('pop_anchor', (e) => {
            pinBottom = e.payload;
        });
        const unlistenText = listen('new_text', (e) => run(e.payload));
        // 截图识别失败走这条，不再退回框选覆盖窗。
        const unlistenErr = listen('recognize_error', (e) => {
            runID++;
            setSource('');
            setItems([]);
            setStatus(e.payload);
        });
        // Switching away is the dismiss gesture. The grace period covers the
        // focus handover right after show(), which lands as a blur first.
        //
        // 但「宽限期内的 blur」不能直接丢掉：如果用户就是在这 300ms 里切走的，
        // 那次 blur 是真的，丢了之后不会再有第二次，面板就永远留在屏幕上了
        // —— 这就是「有时候切窗口它不自动关」的原因，偶发是因为要手快。
        // 改成延到宽限期结束再查一次真实焦点状态：交接抖动那次查出来仍有焦点，
        // 真切走那次查出来没焦点，两种都判对。
        const unlistenBlur = listen('tauri://blur', () => {
            const late = Date.now() - shownAt - 300;
            if (late > 0) {
                void appWindow.hide();
                return;
            }
            setTimeout(async () => {
                if (!(await appWindow.isFocused())) void appWindow.hide();
            }, -late);
        });
        return () => {
            unlistenAnchor.then((f) => f());
            unlistenText.then((f) => f());
            unlistenErr.then((f) => f());
            unlistenBlur.then((f) => f());
        };
    }, []);

    // The window is sized to whatever the content turns out to be, so a two
    // word translation gets a two word panel. Rust only picks the position.
    useEffect(() => {
        const fit = async () => {
            const height = Math.ceil(boxRef.current?.offsetHeight ?? 0);
            if (!height || height === heightRef.current) return;
            heightRef.current = height;
            await appWindow.setSize(new LogicalSize(WIDTH, height));
            // Rust 摆位置时还不知道内容多高。面板在基准上方：钉住底边往上长，
            // 顶部出屏就贴顶；在下方：顶边不动往下长，出屏就贴底往上推。
            const monitor = await currentMonitor();
            if (!monitor) return;
            const position = await appWindow.outerPosition();
            const tall = height * monitor.scaleFactor;
            const y = Math.round(
                pinBottom != null
                    ? Math.max(monitor.position.y, pinBottom - tall)
                    : Math.min(position.y, monitor.position.y + monitor.size.height - tall)
            );
            if (y !== position.y) {
                await appWindow.setPosition(new PhysicalPosition(position.x, y));
            }
        };
        const observer = new ResizeObserver(() => void fit());
        observer.observe(boxRef.current);
        return () => observer.disconnect();
    }, []);

    // 单词的谷歌词典结果窗口里已经拿到了，直接落库：零网络、零 token。
    // 长句先存原文 + 译文立刻返回，AI 拆解在后台回填。
    const collect = async () => {
        if (!source) return;
        const dict = items.find((it) => it.result && typeof it.result === 'object');
        const translation = items.map((it) => plainText(it.result)).find(Boolean) ?? '';
        try {
            await addEntry({
                text: source,
                translation,
                detail: dict && {
                    pronunciations: dict.result.pronunciations ?? [],
                    explanations: dict.result.explanations ?? [],
                },
            });
            setSaved('ok');
        } catch (e) {
            console.error(e);
            setSaved('error');
            // 失败要能重试，所以只有这一支自己退回星星。
            setTimeout(() => setSaved((v) => (v === 'error' ? '' : v)), 1500);
        }
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
            className='relative w-screen rounded-[8px] bg-content1 border-1 border-default-200 overflow-hidden'
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
                onClick={() => armed && void appWindow.hide()}
                onMouseEnter={() => armed && void appWindow.hide()}
            >
                <svg
                    className='w-full h-full text-danger hover:text-danger-600 transition-colors'
                    viewBox='0 0 10 10'
                    fill='currentColor'
                >
                    <polygon points='0,0 10,0 0,10' />
                </svg>
            </div>
            {/* 原文行：语种徽标 + 原文，右边挂和服务行同一套朗读 / 复制。
                按钮不能落进拖拽区 —— data-tauri-drag-region 会把点击整个吞掉，
                所以拖拽区收到文字那一段上，不再罩住整行。
                min-w-0：flex item 默认不肯收缩到内容宽度以下，不加这条 truncate
                就不生效，长原文会把两个按钮挤出面板。 */}
            <div className='px-[8px] pt-[4px] flex items-center gap-[2px] text-[11px] text-default-400'>
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
                {source && (
                    <>
                        <button
                            className='ml-auto shrink-0 hover:text-default-600'
                            aria-label={t('config.wordbook.speak')}
                            title={t('config.wordbook.speak')}
                            onClick={() => speak(source)}
                        >
                            <MdVolumeUp className='text-[12px]' />
                        </button>
                        <button
                            className={`shrink-0 ml-[4px] ${
                                copied === SOURCE_KEY ? 'text-success' : 'hover:text-default-600'
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
                    </>
                )}
            </div>
            <div
                // overflow-x-hidden 是必须的：CSS 里一轴设成非 visible 之后，
                // 另一轴的 visible 会自动变成 auto，所以光写 overflow-y-auto
                // 等于两个方向都能滚，长单词或宽元素就会拖出一条横向滚动条。
                // 左右内边距挪到了每一行上，容器这里不留 —— 见表头那段注释。
                className='overflow-y-auto overflow-x-hidden py-[2px]'
                style={{ maxHeight: MAX_HEIGHT - 46 }}
            >
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
                        <div className='px-[8px] py-[4px] text-[12px] text-danger'>{status}</div>
                    ))}
                {items.map((it) => (
                    <div
                        key={it.key}
                        className='py-[2px]'
                    >
                        {/* 表头行：折叠开关 + 服务名 + 这个服务自己的复制按钮。
                            单服务时也渲染，两种渲染路径不如一种。

                            sticky top-0：滚到下面时表头钉在滚动区顶部（也就是原文
                            那一行下面），不用滚回去就能折叠上面那个服务。多个服务
                            就是一个顶一个的分组表头。

                            背景要铺满整行，否则正文会从缝里穿过去。所以左右内边距
                            放在这一行和正文那一行上，**不放在滚动容器上** ——
                            早先用的是容器 px-[8px] + 表头 -mx-[8px] 抵消，但负边距
                            会连纵向滚动条那 5px 一起吃掉，多出来的宽度正好拖出一条
                            横向滚动条。 */}
                        <div
                            className='sticky top-0 z-10 bg-content1 px-[8px] flex items-center gap-[2px] text-[10px] text-default-400 cursor-pointer select-none'
                            onClick={() => toggle(it.key)}
                        >
                            {collapsed[it.key] ? <MdChevronRight /> : <MdExpandMore />}
                            <span className='truncate'>{it.label}</span>
                            {plainText(it.result) && (
                                <>
                                    {/* 朗读。ml-auto 挂在第一个按钮上，把这一组顶到最右。 */}
                                    <button
                                        className='ml-auto shrink-0 hover:text-default-600'
                                        aria-label={t('config.wordbook.speak')}
                                        title={t('config.wordbook.speak')}
                                        // 不挡住的话，点按钮会顺手把这一段折叠掉。
                                        onClick={(e) => {
                                            e.stopPropagation();
                                            speak(plainText(it.result));
                                        }}
                                    >
                                        <MdVolumeUp className='text-[12px]' />
                                    </button>
                                    <button
                                        className={`shrink-0 ml-[4px] ${
                                            copied === it.key ? 'text-success' : 'hover:text-default-600'
                                        }`}
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
                                </>
                            )}
                        </div>
                        <div className='px-[8px]'>
                            {collapsed[it.key] ? null : it.error ? (
                                <div className='text-[12px] text-danger'>{it.error}</div>
                            ) : it.loading && !it.result ? (
                                <PulseLoader
                                    size={4}
                                    color='#a1a1aa'
                                />
                            ) : typeof it.result === 'string' ? (
                                // break-words：长 URL / 长单词不换行的话会顶宽内容盒，
                                // 上面 overflow-x-hidden 一挡就变成看不见的截断。
                                <div className='text-[13px] whitespace-pre-wrap break-words select-text'>
                                    {it.result}
                                </div>
                            ) : (
                                <DictView data={it.result} />
                            )}
                        </div>
                    </div>
                ))}
            </div>
            {/* 底部只剩收藏：它是「把这条划词整个存进生词本」，本来就不属于
                某一个服务，和上面每服务一个的复制正好分得开。 */}
            <div className='flex justify-end gap-[2px] px-[6px] pb-[3px] pt-[1px]'>
                {/* 收好之后一直停在对勾上，直到换一段划词才退回星星：
                    原来 1.5 秒就复位，看上去像没存进去，很容易再点一次。 */}
                <button
                    disabled={saved === 'ok'}
                    aria-label={t('config.wordbook.title')}
                    title={t('config.wordbook.title')}
                    className={
                        saved === 'ok'
                            ? 'text-success cursor-default'
                            : saved === 'error'
                              ? 'text-danger'
                              : 'text-default-400 hover:text-default-600'
                    }
                    onClick={collect}
                >
                    {saved === 'ok' ? <MdCheck className='text-[13px]' /> : <MdStarBorder className='text-[13px]' />}
                </button>
            </div>
        </div>
    );
}
