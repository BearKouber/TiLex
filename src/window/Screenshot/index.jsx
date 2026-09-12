import React, { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/tauri';

import { recognize } from '../../utils/recognize';
import { listen } from '@tauri-apps/api/event';
import { removeFile } from '@tauri-apps/api/fs';
import { runOcrRequest } from '../../utils/ocr_request';
import { traceOcr } from '../../utils/ocr_diagnostics';
import { useTranslation } from 'react-i18next';

// 框选覆盖窗。Rust 在显示这个窗口之前就已经把整屏抓进内存了，这里只负责画框，
// 回传的是 0..1 的比例 —— 窗口铺满整个虚拟屏，比例乘物理宽高就是物理像素，
// 所以这里一个 devicePixelRatio 都不用碰。细节见 src-tauri/src/screenshot.rs。
//
// 窗口和 pop_result 一样只 hide 不 close，所以每次显示都要把状态清干净。

const MIN = 8; // 比这还小的框当成误点，直接取消

export default function Screenshot() {
    const [sel, setSel] = useState(null); // {x0,y0,x1,y1}，CSS 像素
    const [down, setDown] = useState(false);
    const [msg, setMsg] = useState(''); // 识别中 / 错误
    const { t } = useTranslation();
    const session = useRef({ requestId: 0, active: false });
    const busy = useRef(false);

    // style.css 给 html 加了 10px 圆角（悬浮窗要的），铺满全屏时会缺四个角
    useEffect(() => {
        document.documentElement.style.borderRadius = '0';
    }, []);

    const reset = () => {
        setSel(null);
        setDown(false);
        setMsg('');
    };
    const dismiss = () => {
        const { requestId } = session.current;
        traceOcr('overlay-cancel', { requestId });
        session.current = { requestId, active: false };
        busy.current = false;
        reset();
        void invoke('screenshot_cancel', { requestId });
    };

    // A new capture resets the selection; focus recovery after an error does not.
    useEffect(() => {
        let mounted = true;
        const receive = (value) => {
            if (!mounted || value.requestId <= session.current.requestId) return;
            session.current = value;
            busy.current = false;
            reset();
        };
        const un = listen('screenshot_session', (e) => receive(e.payload));
        // A lazily created webview can miss the first start event. Subscribe before reading.
        void un.then(() => invoke('screenshot_current')).then(receive);
        const keydown = (e) => { if (e.key === 'Escape') dismiss(); };
        document.addEventListener('keydown', keydown);
        return () => {
            mounted = false;
            dismiss();
            void un.then((f) => f());
            document.removeEventListener('keydown', keydown);
        };
    }, []);

    const finish = async (e) => {
        setDown(false);
        if (!down || !sel || busy.current || !session.current.active) return;
        const box = rectOf({ ...sel, x1: e.clientX, y1: e.clientY });
        if (box.w < MIN || box.h < MIN) return dismiss();
        const { clientWidth: cw, clientHeight: ch } = document.documentElement;
        const { requestId } = session.current;
        const current = () => session.current.active && session.current.requestId === requestId;
        busy.current = true;
        try {
            await runOcrRequest({
                current,
                // Native commands also check ownership, including time spent in the IPC queue.
                hide: () => invoke('screenshot_overlay', { requestId, visible: false }),
                crop: () => invoke('crop_region', {
                    requestId, left: box.l / cw, top: box.t / ch,
                    right: (box.l + box.w) / cw, bottom: (box.t + box.h) / ch,
                }),
                show: ({ left, top, right, bottom }) => invoke('show_pop_result', {
                    requestId, left, top, right, bottom, text: '',
                }),
                recognize,
                // `error` is reserved by Tauri v1 for its numeric rejection callback.
                publish: (text, isError) => invoke('screenshot_publish', { requestId, text, isError }),
                restore: async (message) => {
                    if (!current()) return;
                    setMsg(message);
                    await invoke('screenshot_overlay', { requestId, visible: true });
                },
                reset,
                cleanup: (path) => removeFile(path).catch(() => {}),
                noText: t('config.service.no_text'),
                failureText: t('config.recognize.failed'),
                trace: (event, details) => traceOcr(event, { requestId, ...details }),
            });
        } catch {
            // The session may have been cancelled while a guarded native command was queued.
        } finally {
            if (current()) busy.current = false;
        }
    };

    const box = sel && rectOf(sel);

    return (
        <div
            className='fixed inset-0 cursor-crosshair select-none overflow-hidden'
            style={{ background: box ? 'transparent' : 'rgba(0,0,0,0.45)' }}
            onMouseDown={(e) => {
                if (e.button !== 0) return dismiss(); // 右键取消
                setMsg('');
                setDown(true);
                setSel({ x0: e.clientX, y0: e.clientY, x1: e.clientX, y1: e.clientY });
            }}
            onMouseMove={(e) => down && setSel((s) => ({ ...s, x1: e.clientX, y1: e.clientY }))}
            onMouseUp={finish}
        >
            {box && (
                // 选区本身不铺底色，用一圈大到出屏的投影把外面压暗，
                // 这样「亮的是选区」和「暗的是其余」只需要一个元素
                <div
                    className='absolute border border-white/80'
                    style={{
                        left: box.l,
                        top: box.t,
                        width: box.w,
                        height: box.h,
                        boxShadow: '0 0 0 100vmax rgba(0,0,0,0.45)',
                    }}
                />
            )}
            <div className='absolute inset-x-0 top-8 flex justify-center'>
                <span className='rounded bg-black/70 px-3 py-1 text-sm text-white'>{msg || t('recognize.hint')}</span>
            </div>
        </div>
    );
}

function rectOf({ x0, y0, x1, y1 }) {
    return {
        l: Math.min(x0, x1),
        t: Math.min(y0, y1),
        w: Math.abs(x1 - x0),
        h: Math.abs(y1 - y0),
    };
}
