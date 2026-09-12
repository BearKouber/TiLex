// 框选截图。这里只干「原生」那一半：抓屏、画框、裁图存盘。
//
// 认字交给前端 —— 识别服务有微信 OCR 和 Umi-OCR 两家，后者要读 store 里的地址、
// 要发 HTTP，那些本来就是前端在管的事（翻译服务也是这么分的）。所以这里返回裁好
// 的 PNG 路径，前端认完字再调 show_pop_result 把文字送进悬浮面板。
//
// 顺序：先把整个虚拟屏 BitBlt 进内存，再弹覆盖窗。识别用的像素永远来自「弹窗之前」
// 抓的那一份，所以覆盖窗自己绝不会被拍进去。
//
// 覆盖窗是透明的，没有把那一帧写成 PNG 再当背景贴回去 —— 全屏 PNG 编一次解一次
// 要好几百毫秒，覆盖窗就慢半拍才出来。代价是拖框期间用户看到的是活的桌面而不是
// 冻结的那一帧，只有画面正在动的时候才看得出区别。
//
// 前端回传的是 0..1 的比例，不是像素：覆盖窗铺满整个虚拟屏，比例乘物理宽高就是
// 物理像素。所以这条链路上没有 devicePixelRatio，多屏各自不同的缩放也不用分屏算。

use once_cell::sync::Lazy;
use std::sync::Mutex;
use tauri::Manager;

#[derive(Clone, Copy, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    request_id: u64,
    active: bool,
}

impl Session {
    fn advance(&mut self, active: bool) -> u64 {
        self.request_id += 1;
        self.active = active;
        self.request_id
    }

    fn accepts(&self, request_id: u64) -> bool {
        self.active && self.request_id == request_id
    }
}

// Serialize native side effects with new captures/cancellation. JS checks alone cannot
// guard commands already queued across IPC, nor the shared full-screen buffer.
static SESSION: Lazy<Mutex<Session>> = Lazy::new(|| Mutex::new(Session::default()));

fn announce(session: Session) {
    if let Some(app) = crate::APP.get() {
        let _ = app.emit_all("screenshot_session", session);
    }
}

pub fn with_current<T>(request_id: u64, action: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let session = SESSION.lock().map_err(|e| e.to_string())?;
    if !session.accepts(request_id) { return Err("Screenshot superseded".into()); }
    action()
}

// Normal text selection also owns the result panel and supersedes any pending OCR.
pub fn with_selection(action: impl FnOnce()) {
    if let Ok(mut session) = SESSION.lock() {
        session.advance(false);
        announce(*session);
        action();
    }
}

#[tauri::command(async)]
pub fn screenshot_current() -> Result<Session, String> {
    SESSION.lock().map(|session| *session).map_err(|e| e.to_string())
}

#[tauri::command(async)]
pub fn screenshot_is_current(request_id: u64) -> bool {
    SESSION.lock().map(|session| session.accepts(request_id)).unwrap_or(false)
}

#[tauri::command(async)]
pub fn screenshot_cancel(request_id: u64) -> Result<(), String> {
    let mut session = SESSION.lock().map_err(|e| e.to_string())?;
    if !session.accepts(request_id) { return Ok(()); }
    session.advance(false);
    announce(*session);
    overlay_visibility(false)
}

fn overlay_visibility(visible: bool) -> Result<(), String> {
    if let Some(window) = crate::APP.get().and_then(|app| app.get_window("screenshot")) {
        if visible { window.show() } else { window.hide() }.map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command(async)]
pub fn screenshot_overlay(request_id: u64, visible: bool) -> Result<(), String> {
    with_current(request_id, || overlay_visibility(visible))
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrText {
    pub request_id: u64,
    pub text: String,
}

#[tauri::command(async)]
pub fn screenshot_publish(request_id: u64, text: String, is_error: bool) -> Result<(), String> {
    with_current(request_id, || {
        let window = crate::APP.get().and_then(|app| app.get_window("pop_result")).ok_or("No result window")?;
        window.emit(if is_error { "recognize_error" } else { "new_text" }, OcrText { request_id, text })
            .map_err(|e| e.to_string())
    })
}

fn region_filename() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    format!("ocr_region_{}_{}.png", std::process::id(), SERIAL.fetch_add(1, Ordering::Relaxed))
}

#[cfg(windows)]
mod imp {
    use crate::APP;
    use log::info;
    use once_cell::sync::Lazy;
    use std::io::Write;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use super::Region;
    use tauri::{Manager, PhysicalPosition, PhysicalSize};

    const LABEL: &str = "screenshot";

    /// 抓下来的整个虚拟屏。x/y 是虚拟屏左上角在桌面坐标系里的位置（多屏时可以是负数）。
    struct Shot {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        rgb: Vec<u8>,
    }

    static SHOT: Lazy<Mutex<Option<Shot>>> = Lazy::new(|| Mutex::new(None));

    fn capture() -> Result<Shot, String> {
        use windows::core::w;
        use windows::Win32::Graphics::Gdi::{
            BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateDCW, DeleteDC, DeleteObject,
            GetDIBits, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, CAPTUREBLT,
            DIB_RGB_COLORS, SRCCOPY,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
            SM_YVIRTUALSCREEN,
        };

        unsafe {
            let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
            let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
            let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
            let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
            if w <= 0 || h <= 0 {
                return Err(format!("虚拟屏尺寸不对：{w}x{h}"));
            }

            let screen = CreateDCW(w!("DISPLAY"), None, None, None);
            let mem = CreateCompatibleDC(screen);
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, bmp);
            // CAPTUREBLT 才带得上分层窗口（输入法候选框、半透明面板都是这种）
            let blt = BitBlt(mem, 0, 0, w, h, screen, x, y, SRCCOPY | CAPTUREBLT).is_ok();

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
            let mut bgra = vec![0u8; (w as usize) * (h as usize) * 4];
            let lines = GetDIBits(
                mem,
                bmp,
                0,
                h as u32,
                Some(bgra.as_mut_ptr() as *mut _),
                &mut info,
                DIB_RGB_COLORS,
            );

            SelectObject(mem, old);
            let _ = DeleteObject(bmp);
            let _ = DeleteDC(mem);
            let _ = DeleteDC(screen);

            if !blt || lines == 0 {
                return Err("抓屏失败（BitBlt / GetDIBits）".into());
            }
            // GDI 的 alpha 通道是没定义的垃圾值，丢掉，只留 RGB
            let rgb = bgra
                .chunks_exact(4)
                .flat_map(|p| [p[2], p[1], p[0]])
                .collect();
            Ok(Shot { x, y, w, h, rgb })
        }
    }

    /// 从整屏缓冲里裁一块存成 PNG。压缩级别用 Fast：这是喂给 OCR 的中间文件，
    /// 体积没人看，快比小重要。
    /// 裁出来的图落哪：app cache，不是 %TEMP%。前端 fs 的 allowlist scope 只放行
    /// $APPCONFIG/** 和 $APPCACHE/**，落 %TEMP% 里 Google 那条读不到这张图。
    fn region_path() -> Result<PathBuf, String> {
        let app = APP.get().ok_or("APP 还没初始化")?;
        let dir = tauri::api::path::app_cache_dir(&app.config()).ok_or("拿不到 app cache 目录")?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("建不出缓存目录：{e}"))?;
        Ok(dir.join(super::region_filename()))
    }

    // 路径由调用方给：算路径要 APP，而裁剪 + PNG 编码这段是真正会写错的地方，
    // 拆开之后 cargo test 里不用起一个 tauri app 也能测它。
    fn crop_png(shot: &Shot, l: i32, t: i32, w: i32, h: i32, path: &Path) -> Result<(), String> {
        let file = std::fs::File::create(path).map_err(|e| format!("写不出临时图：{e}"))?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut header = enc.write_header().map_err(|e| e.to_string())?;
        let mut writer = header.stream_writer().map_err(|e| e.to_string())?;
        let stride = shot.w as usize * 3;
        for row in 0..h as usize {
            let from = (t as usize + row) * stride + l as usize * 3;
            writer
                .write_all(&shot.rgb[from..from + w as usize * 3])
                .map_err(|e| e.to_string())?;
        }
        writer.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 覆盖窗只建一次，之后只 show/hide —— tauri v1 在主线程上关窗会 panic。
    // ponytail: 第一次点托盘才建窗，webview 起来那 200~400ms 里屏幕上什么都没有。
    // 嫌慢就照 pop_result 的样子在 setup 里先建好藏着。
    fn overlay() -> Result<tauri::Window, String> {
        let app = APP.get().ok_or("APP 还没初始化")?;
        if let Some(v) = app.get_window(LABEL) {
            return Ok(v);
        }
        tauri::WindowBuilder::new(app, LABEL, tauri::WindowUrl::App("index.html".into()))
            .visible(false)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .skip_taskbar(true)
            .resizable(false)
            .additional_browser_args("--disable-web-security")
            .build()
            .map_err(|e| e.to_string())
    }

    /// 托盘点进来的入口：抓屏 → 铺满虚拟屏的覆盖窗 → 交给前端拖框。
    pub fn start() -> Result<(), String> {
        // Discard the previous pixels even when a fresh capture fails.
        *SHOT.lock().map_err(|e| e.to_string())? = None;
        let shot = capture()?;
        let (x, y, w, h) = (shot.x, shot.y, shot.w, shot.h);
        info!("Screenshot: captured {}x{} at ({}, {})", w, h, x, y);
        *SHOT.lock().unwrap() = Some(shot);

        let window = overlay()?;
        window.set_position(PhysicalPosition::new(x, y)).map_err(|e| e.to_string())?;
        window.set_size(PhysicalSize::new(w as u32, h as u32)).map_err(|e| e.to_string())?;
        window.show().map_err(|e| e.to_string())?;
        window.set_focus().map_err(|e| e.to_string())?;
        Ok(())
    }

    /// 选区用 0..1 的比例传进来，见本文件顶部。裁好图返回路径，认字是前端的事。
    /// 连同选区矩形的物理坐标一起返回 —— 前端认完字原样交给 show_pop_result 定位。
    pub fn crop_region(left: f64, top: f64, right: f64, bottom: f64) -> Result<Region, String> {
        let guard = SHOT.lock().unwrap();
        let shot = guard.as_ref().ok_or("还没抓屏")?;

        let px = |f: f64, max: i32| (f * max as f64).round().clamp(0.0, max as f64) as i32;
        let (l, t) = (px(left, shot.w), px(top, shot.h));
        let (w, h) = (px(right, shot.w) - l, px(bottom, shot.h) - t);
        if w < 4 || h < 4 {
            return Err("选区太小了".into());
        }

        let path = region_path()?;
        if let Err(error) = crop_png(shot, l, t, w, h, &path) {
            let _ = std::fs::remove_file(&path);
            return Err(error);
        }
        Ok(Region {
            path: path.to_string_lossy().into_owned(),
            left: shot.x + l,
            top: shot.y + t,
            right: shot.x + l + w,
            bottom: shot.y + t + h,
        })
    }

    #[cfg(test)]
    mod tests {
        // 真抓一次屏再裁一块，把 PNG 解回来核对尺寸 —— 裁剪的行偏移和 PNG 的
        // 宽高只要错一点，这里就对不上。没有桌面会话时 capture() 失败，跳过。
        #[test]
        fn crops_the_captured_screen() {
            let Ok(shot) = super::capture() else {
                eprintln!("skipped: no desktop session");
                return;
            };
            assert_eq!(shot.rgb.len(), shot.w as usize * shot.h as usize * 3);
            let (w, h) = (100.min(shot.w), 50.min(shot.h));
            let path = std::env::temp_dir().join("tilex_crop_test.png");
            super::crop_png(&shot, 10, 10, w, h, &path).unwrap();
            let dec = png::Decoder::new(std::fs::File::open(&path).unwrap());
            let mut reader = dec.read_info().unwrap();
            let mut buf = vec![0; reader.output_buffer_size()];
            let info = reader.next_frame(&mut buf).unwrap();
            assert_eq!((info.width, info.height), (w as u32, h as u32));
            assert_eq!(info.buffer_size(), w as usize * h as usize * 3);
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn start() -> Result<(), String> { Err("仅 Windows".into()) }
    pub fn crop_region(_l: f64, _t: f64, _r: f64, _b: f64) -> Result<super::Region, String> {
        Err("仅 Windows".into())
    }
}

/// 裁好的那张图，外加选区矩形（物理坐标），悬浮面板照它摆。
#[derive(serde::Serialize)]
pub struct Region {
    pub path: String,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// 托盘菜单点「截图翻译」。抓屏要几十毫秒，别占着事件循环。
pub fn start_screenshot() {
    // Never acquire this mutex on the UI thread: a worker holding it may be
    // waiting for a Tauri window getter. Invalidate before capture, not on focus.
    std::thread::spawn(|| {
        match SESSION.lock() {
            Ok(mut session) => {
                session.advance(true);
                announce(*session);
                if let Err(error) = imp::start() {
                    log::error!("Screenshot: {error}");
                    session.advance(false);
                    announce(*session);
                    let _ = overlay_visibility(false);
                }
            }
            Err(error) => log::error!("Screenshot session: {error}"),
        }
    });
}

#[tauri::command(async)]
pub fn crop_region(request_id: u64, left: f64, top: f64, right: f64, bottom: f64) -> Result<Region, String> {
    with_current(request_id, || imp::crop_region(left, top, right, bottom))
}

#[cfg(test)]
mod session_tests {
    #[test]
    fn publication_preserves_tauri_callback_envelope() {
        // Exercise Tauri's real deserializer: an `error: bool` business argument
        // replaces the callback ID and rejects the entire message before dispatch.
        for is_error in [false, true] {
            let payload = serde_json::json!({
                "cmd": "screenshot_publish", "callback": 100, "error": 101,
                "requestId": 7, "text": "OCR result", "isError": is_error,
            });
            let parsed: tauri::InvokePayload = serde_json::from_value(payload.clone()).unwrap();
            assert_eq!(parsed.error.0, 101);
            assert_eq!(parsed.inner["isError"], is_error);
            assert!(parsed.inner.get("error").is_none());

            let mut broken = payload;
            broken["error"] = is_error.into();
            assert!(serde_json::from_value::<tauri::InvokePayload>(broken).is_err());
        }
    }

    #[test]
    fn newer_capture_cancel_and_selection_reject_old_work() {
        let mut session = super::Session::default();
        let a = session.advance(true);
        assert!(session.accepts(a));
        let b = session.advance(true);
        assert!(!session.accepts(a));
        assert!(session.accepts(b));
        session.advance(false);
        assert!(!session.accepts(b));
    }

    #[test]
    fn crops_have_independent_paths() {
        assert_ne!(super::region_filename(), super::region_filename());
    }

    #[test]
    fn native_guard_blocks_stale_actions_and_stale_cancel() {
        let (old, current) = {
            let mut session = super::SESSION.lock().unwrap();
            (session.advance(true), session.advance(true))
        };
        let mut called = false;
        assert!(super::with_current(old, || { called = true; Ok(()) }).is_err());
        assert!(!called);
        super::screenshot_cancel(old).unwrap();
        assert!(super::screenshot_is_current(current));
        super::with_current(current, || { called = true; Ok(()) }).unwrap();
        assert!(called);
        super::with_selection(|| {});
        assert!(!super::screenshot_is_current(current));
    }
}
