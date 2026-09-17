use std::ffi::c_void;
use std::time::Instant;

use slint::{Rgba8Pixel, SharedPixelBuffer};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
    CreateCompatibleDC, CreateDCW, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, SRCCOPY,
    SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};
use windows::core::w;

use super::super::Shot;
use crate::error::Error;

/// 抓取整个虚拟屏并直接写入 `SharedPixelBuffer`。
/// 包含多显示器虚拟桌面；耗时约数十毫秒，不可在 UI 线程调用。
pub fn capture_screen() -> Result<Shot, Error> {
    let start = Instant::now();

    // SAFETY: GetSystemMetrics 为只读无指针 Win32 API。
    let (x, y, w, h) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    };
    if w <= 0 || h <= 0 {
        return Err(Error::Platform(format!(
            "invalid virtual screen size: {w}x{h}"
        )));
    }

    // 峰值内存优化：只分配一份全屏 SharedPixelBuffer，GetDIBits 直接写进它的底层字节缓冲，
    // 随后原地 swap(0, 2) 将 BGRA 转换为 RGBA 并将 alpha 刷为 255，不分配第二份大 Vec。
    let mut pixels = SharedPixelBuffer::<Rgba8Pixel>::new(w as u32, h as u32);

    let (blt, lines) = {
        // SAFETY: CreateDCW 参数全为有效只读常量或 None；mem/bmp 为兼容设备上下文与位图。
        let (screen, mem, bmp, old) = unsafe {
            let screen = CreateDCW(w!("DISPLAY"), None, None, None);
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bmp);
            (screen, mem, bmp, old)
        };

        // CAPTUREBLT 才带得上分层窗口（输入法候选框、半透明面板都是这种）
        // SAFETY: screen, mem, bmp 均为刚创建的有效 GDI 句柄。
        let blt = unsafe { BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT).is_ok() };

        let mut lines = 0;
        if blt {
            // biHeight 取负 = 自上而下，省得再把行倒过来
            let mut info = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            // SAFETY: pixels 是刚为虚拟屏 (w x h) 分配的 SharedPixelBuffer，make_mut_bytes 长度为
            // w * h * 4 字节，与 BITMAPINFOHEADER 声明的 32bpp、自上而下 (-h) 格式完全一致，指针在
            // GetDIBits 期间独占有效。
            lines = unsafe {
                GetDIBits(
                    mem,
                    bmp,
                    0,
                    h as u32,
                    Some(pixels.make_mut_bytes().as_mut_ptr().cast::<c_void>()),
                    &mut info,
                    DIB_RGB_COLORS,
                )
            };
        }

        // GDI 资源无论成功失败均按 SelectObject(old) -> DeleteObject(bmp) -> DeleteDC(mem) -> DeleteDC(screen) 逆序释放。
        // SAFETY: 句柄来自本函数刚获取的资源，按 Windows API 约定逆序释放。
        unsafe {
            SelectObject(mem, old);
            let _ = DeleteObject(bmp); // ignore: GDI 资源清理失败无法恢复，进程退出时系统回收
            let _ = DeleteDC(mem); // ignore: 同上
            let _ = DeleteDC(screen); // ignore: 同上
        }

        (blt, lines)
    };

    if !blt || lines == 0 {
        return Err(Error::Platform(
            "capture screen failed (BitBlt / GetDIBits)".into(),
        ));
    }

    // 原地把 BGRA 转成 RGBA，且将 GDI 未定义的 alpha 垃圾值刷成 255
    for p in pixels.make_mut_bytes().as_chunks_mut::<4>().0 {
        p.swap(0, 2);
        p[3] = 255;
    }

    let elapsed = start.elapsed();
    log::info!("Screenshot: captured {w}x{h} at ({x}, {y}) in {elapsed:?}");

    Ok(Shot { x, y, pixels })
}
