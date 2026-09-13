import assert from 'node:assert/strict';
import { createPopResultSizing } from './pop_result_sizing.js';

function deferred() {
    let resolve;
    const promise = new Promise((done) => {
        resolve = done;
    });
    return { promise, resolve };
}

// 所有异步链都是微任务，setTimeout 0 之后一定跑完。
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

function fixture(overrides = {}) {
    const state = { height: 80, nativeHeight: 80, position: { x: 30, y: 620 }, sizes: [], positions: [], paints: 0 };
    const monitor = { position: { x: 0, y: 0 }, size: { width: 1000, height: 1000 }, scaleFactor: 1 };
    const sizing = createPopResultSizing({
        width: 320,
        measure: () => state.height,
        setSize: async (width, height) => {
            assert.equal(width, 320);
            state.sizes.push(height);
            if (overrides.setSize) await overrides.setSize(height);
            state.nativeHeight = height;
        },
        currentMonitor: overrides.currentMonitor ?? (async () => monitor),
        outerPosition: async () => ({ ...state.position }),
        setPosition: async (x, y) => {
            state.positions.push(y);
            state.position = { x, y };
        },
        repaint: () => state.paints++,
    });
    return { sizing, state, monitor };
}

// 串行：原生调用没回来之前不开第二个，中间的 120 被合并掉，只追最新的 240。
{
    const waiting = deferred();
    let first = true;
    const f = fixture({
        setSize: () => {
            if (!first) return;
            first = false;
            return waiting.promise;
        },
    });
    f.state.position.y = 600;
    f.sizing.setAnchor(700);
    f.state.height = 120;
    f.sizing.request();
    f.state.height = 240;
    f.sizing.request();
    assert.deepEqual(f.state.sizes, [80]);
    waiting.resolve();
    await settle();
    assert.deepEqual(f.state.sizes, [80, 240]);
    assert.deepEqual(f.state.positions, [620, 460]);
    assert.equal(f.state.paints, 2, 'every applied size forces a frame');
    f.sizing.request();
    await settle();
    assert.equal(f.state.sizes.length, 2, 'same height is not resubmitted');
    assert.equal(f.state.paints, 2);
}

// 失败不记缓存：同一高度下次还能重试，失败那次不 repaint。
{
    let failing = true;
    const f = fixture({
        setSize: async () => {
            if (failing) throw new Error('transient');
        },
    });
    f.state.height = 240;
    f.sizing.request();
    await settle();
    assert.equal(f.state.paints, 0);
    failing = false;
    f.sizing.request();
    await settle();
    assert.deepEqual(f.state.sizes, [240, 240]);
    assert.equal(f.state.paints, 1);
}

// 同高度重新弹出、锚点变化都要重新对位置。
{
    const f = fixture();
    f.state.height = 240;
    f.sizing.setAnchor(700);
    await settle();
    f.state.position.y = 600; // Rust 把复用的窗口挪到了新位置
    f.sizing.refresh();
    await settle();
    assert.equal(f.state.position.y, 460);
    f.sizing.setAnchor(500);
    await settle();
    assert.equal(f.state.position.y, 260);
    assert.deepEqual(f.state.sizes, [240, 240, 240]);
}

// 飞行中换了锚点：旧请求不能再写旧底边，也不 repaint。
{
    const waiting = deferred();
    let first = true;
    let f;
    f = fixture({
        currentMonitor: async () => {
            if (first) {
                first = false;
                await waiting.promise;
            }
            return f.monitor;
        },
    });
    f.state.height = 240;
    f.sizing.setAnchor(700);
    f.sizing.setAnchor(500);
    waiting.resolve();
    await settle();
    assert.deepEqual(f.state.positions, [260]);
    assert.equal(f.state.paints, 1);
}

// 物理像素下的底边钉住、负坐标显示器、上下贴边。
{
    const f = fixture();
    f.monitor.scaleFactor = 1.5;
    f.monitor.position.y = -500;
    f.state.height = 240;
    f.sizing.setAnchor(-200);
    await settle();
    assert.equal(f.state.position.y, -500);
    f.state.position.y = 400;
    f.sizing.setAnchor(null);
    await settle();
    assert.equal(f.state.position.y, 140);
    f.state.position.y = 30;
    f.state.height = 80;
    f.sizing.request();
    await settle();
    assert.equal(f.state.position.y, 30, 'downward shrink preserves the top edge');
}

// 量不出高度（0 / NaN / 负数）就跳过，之后的正常值照常提交。
{
    const f = fixture();
    for (const height of [0, NaN, -1]) {
        f.state.height = height;
        f.sizing.request();
    }
    await settle();
    assert.deepEqual(f.state.sizes, []);
    f.state.height = 80.2;
    f.sizing.request();
    await settle();
    assert.deepEqual(f.state.sizes, [81]);
}

// invalidate 暂停到下一次 refresh；dispose 之后永远不动。
for (const method of ['invalidate', 'dispose']) {
    const f = fixture();
    f.sizing[method]();
    f.state.height = 240;
    f.sizing.request();
    await settle();
    assert.deepEqual(f.state.sizes, []);
    f.sizing.refresh();
    await settle();
    assert.deepEqual(f.state.sizes, method === 'dispose' ? [] : [240]);
}

// clearAnchor：拖拽后清除 pinBottom，后续高度变化不再吸附原锚点。
{
    const f = fixture();
    f.sizing.setAnchor(700);
    f.state.height = 200;
    f.sizing.request();
    await settle();
    assert.equal(f.state.position.y, 500);

    // 用户拖动到 300 并清除锚点
    f.sizing.clearAnchor();
    f.state.position.y = 300;
    f.state.height = 250;
    f.sizing.request();
    await settle();
    assert.equal(f.state.position.y, 300, 'clearing anchor preserves the dragged top position instead of snapping to pinBottom');
}

console.log('PopResult sizing: serial, retry, anchor, position, lifecycle and repaint tests passed');
