import React, { useEffect, useState } from 'react';
import { appWindow } from '@tauri-apps/api/window';
import { invoke } from '@tauri-apps/api/tauri';

import { recognize } from '../../utils/recognize';
import { emit, listen } from '@tauri-apps/api/event';
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
        reset();
        void appWindow.hide();
    };

    // Esc 关窗是 App.jsx 上那个全局监听干的，状态清理挂在「又被显示出来」这一刻，
    // 这样不管上次是怎么退出的，下次打开都是干净的。
    useEffect(() => {
        const un = listen('tauri://focus', reset);
        return () => void un.then((f) => f());
    }, []);

    const finish = async (e) => {
        setDown(false);
        if (!sel) return;
        const box = rectOf({ ...sel, x1: e.clientX, y1: e.clientY });
        if (box.w < MIN || box.h < MIN) return dismiss();
        const { clientWidth: cw, clientHeight: ch } = document.documentElement;
        // 先收覆盖窗，再识别 —— 顺序不能反。反过来的话结果面板已经弹出来了，
        // 覆盖窗这时候 hide，Windows 要重新分配焦点，面板立刻吃到一个 blur，
        // 按「失焦就消失」的规矩把自己关掉 —— 就是那个「闪一下直接退了」。
        // 顺带识别那半秒屏幕是干净的，不用一直压着一层灰。
        await appWindow.hide();
        // 面板先亮出来（空文本 = 识别中），认完字再把结果 emit 过去。等认完再弹
        // 的话，Umi 认一屏字要好几秒，那几秒屏幕上什么都没有，像卡死了。
        let popped = false;
        try {
            // Rust 只管裁图，认字走前端的识别服务（微信 OCR / Umi-OCR），
            // 因为后者要读 store 里的 key、要发 HTTP —— 和翻译服务一个分法。
            const region = await invoke('crop_region', {
                left: box.l / cw,
                top: box.t / ch,
                right: (box.l + box.w) / cw,
                bottom: (box.t + box.h) / ch,
            });
            const { left, top, right, bottom } = region;
            await invoke('show_pop_result', { left, top, right, bottom, text: '' });
            popped = true;
            const text = await recognize(region.path);
            if (!text) throw new Error(t('config.service.no_text'));
            // 面板已经开着了，第二次不能再走 show_pop_result —— 那会重新定位、
            // 重新抢焦点。全局 emit 直接送进它的 new_text 监听就够了。
            await emit('new_text', text);
            reset();
        } catch (err) {
            // 取 message：识别服务抛的已经是给人看的话了，加个 "Error:" 前缀反而难看。
            const message = err?.message ?? String(err);
            // 错误也进面板。原来是把覆盖窗叫回来显示 —— 但那样一个识别服务都没开
            // 的时候，每次划完框都退回框选状态，看着像卡在截屏里出不去。
            if (popped) await emit('recognize_error', message);
            else {
                setMsg(message);
                void appWindow.show();
            }
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
