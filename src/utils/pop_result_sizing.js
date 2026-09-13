// 窗口高度跟着内容走。原生调用都是异步的，所以串行执行、只追最新高度；
// 每个高度完整设好（尺寸 + 位置）后调一次 repaint，原因见 PopResult 里 repaint 的注释。
export function createPopResultSizing({ width, measure, setSize, currentMonitor, outerPosition, setPosition, repaint }) {
    // Rust 摆放时面板在基准上方，就是面板底边的物理 y（内容变高时钉住它往上长）；
    // 在下方是 null（顶边不动往下长）。
    let pinBottom = null;
    let generation = 0;
    // invalidate 之后暂停（截图会话换了，旧内容别再撑窗口），等下一次 refresh 恢复。
    let active = true;
    let disposed = false;
    // 本代次最后一次完整设好的高度。失败不记，下一次内容变化或弹出会再试。
    let applied = 0;
    let running = false;
    let again = false;

    const request = async () => {
        if (!active) return;
        if (running) {
            again = true;
            return;
        }
        const height = Math.ceil(measure());
        if (!Number.isFinite(height) || height <= 0 || height === applied) return;
        const current = generation;
        running = true;
        try {
            await setSize(width, height);
            const monitor = await currentMonitor();
            const position = await outerPosition();
            if (current !== generation || !monitor) return;
            // 原生坐标是物理像素。在上方：钉住底边往上长，顶部出屏就贴顶；
            // 在下方：顶边不动往下长，出屏就贴底往上推。
            const tall = height * monitor.scaleFactor;
            const y = Math.round(
                pinBottom != null
                    ? Math.max(monitor.position.y, pinBottom - tall)
                    : Math.min(position.y, monitor.position.y + monitor.size.height - tall)
            );
            if (y !== position.y) await setPosition(position.x, y);
            if (current !== generation) return;
            applied = height;
            repaint();
        } catch {
            // 同上：不记 applied。
        } finally {
            running = false;
            if (again) {
                again = false;
                request();
            }
        }
    };
    const invalidate = () => {
        generation++;
        applied = 0;
        active = false;
    };
    const refresh = () => {
        if (disposed) return;
        invalidate();
        active = true;
        request();
    };
    return {
        request,
        refresh,
        setAnchor(bottom) {
            pinBottom = bottom;
            refresh();
        },
        invalidate,
        dispose() {
            invalidate();
            disposed = true;
        },
    };
}
