use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::platform::Shot;

/// 每次裁图一个独立文件名：`ocr_region_<pid>_<序号>.png`。
/// 共用一个固定名字会让两次截图串图（旧 `ocr-request-ownership.md` 的 Bad case）。
fn region_filename() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    format!(
        "ocr_region_{}_{}.png",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    )
}

/// 缓存目录下的一个新临时图路径。目录不存在会建出来。
pub fn region_path() -> Result<PathBuf, Error> {
    let dir = crate::platform::cache_dir()?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join(region_filename()))
}

/// 裁好的那张图，外加选区矩形（桌面物理坐标）。结果浮窗照它摆。
pub struct Region {
    pub path: PathBuf,
    pub rect: crate::platform::geometry::Rect,
}

/// 选区用 0..1 的比例给，返回裁好的 PNG 路径和选区在桌面坐标系里的物理矩形。
/// 比例不是像素：遮罩铺满整个虚拟屏，比例乘物理宽高就是物理像素，多屏各自不同的缩放不用分屏算
/// （旧版文件头 `screenshot.rs:14-15`）。
pub fn crop_region(
    shot: &Shot,
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
) -> Result<Region, Error> {
    let width = shot.pixels.width() as i32;
    let height = shot.pixels.height() as i32;

    let px = |f: f64, max: i32| (f * max as f64).round().clamp(0.0, max as f64) as i32;
    let (l, t) = (px(left, width), px(top, height));
    let (w, h) = (px(right, width) - l, px(bottom, height) - t);
    if w < 4 || h < 4 {
        return Err(Error::Platform("selection too small".into()));
    }

    let path = region_path()?;
    if let Err(error) = crop_png(shot, l as u32, t as u32, w as u32, h as u32, &path) {
        let _ = std::fs::remove_file(&path); // ignore: 裁图失败清理残留文件
        return Err(error);
    }

    log::info!("Screenshot: crop {w}x{h}");
    Ok(Region {
        path,
        rect: crate::platform::geometry::Rect {
            l: shot.x + l,
            t: shot.y + t,
            r: shot.x + l + w,
            b: shot.y + t + h,
        },
    })
}

/// 从全屏缓冲中裁切指定矩形并编码为 PNG 写入目标路径。
/// 保持为不碰文件系统以外的纯函数，按行抽取 RGB 像素复用单行缓冲，跳过 alpha 通道。
fn crop_png(shot: &Shot, l: u32, t: u32, w: u32, h: u32, path: &Path) -> Result<(), Error> {
    use std::io::Write;

    let pw = shot.pixels.width();
    let ph = shot.pixels.height();
    if l.checked_add(w).is_none_or(|r| r > pw) || t.checked_add(h).is_none_or(|b| b > ph) {
        return Err(Error::Platform("crop rectangle out of bounds".into()));
    }

    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.set_compression(png::Compression::Fast);
    let mut header = enc
        .write_header()
        .map_err(|e| Error::Platform(e.to_string()))?;
    let mut writer = header
        .stream_writer()
        .map_err(|e| Error::Platform(e.to_string()))?;

    let stride = (pw as usize) * 4;
    let raw = shot.pixels.as_bytes();
    let mut row_rgb = Vec::with_capacity(w as usize * 3);
    for row in 0..h as usize {
        let row_start = (t as usize + row) * stride + (l as usize * 4);
        let row_end = row_start + (w as usize * 4);
        row_rgb.clear();
        for chunk in raw[row_start..row_end].as_chunks::<4>().0 {
            row_rgb.extend_from_slice(&chunk[..3]);
        }
        writer.write_all(&row_rgb)?;
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 两个用到缓存目录的测试要串起来跑：`cargo test` 默认多线程，
    /// 否则 `rejects_a_selection_smaller_than_four_pixels` 的目录快照会拍到别的测试的临时图。
    static CACHE_DIR: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn crops_have_independent_paths() {
        assert_ne!(region_filename(), region_filename());
    }

    #[test]
    #[allow(clippy::print_stderr, reason = "测试跳过或打印诊断输出")]
    fn crops_the_captured_screen() {
        let Ok(shot) = crate::platform::capture_screen() else {
            eprintln!("skipped: no desktop session");
            return;
        };
        let w = shot.pixels.width();
        let h = shot.pixels.height();
        assert_eq!(shot.pixels.as_bytes().len(), w as usize * h as usize * 4);

        let (cw, ch) = (100.min(w), 50.min(h));
        let path = std::env::temp_dir().join(format!("tilex_crop_test_{}.png", std::process::id()));
        if let Err(e) = super::crop_png(&shot, 10, 10, cw, ch, &path) {
            panic!("crop_png failed: {e}");
        }

        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => panic!("open failed: {e}"),
        };
        let dec = png::Decoder::new(std::io::BufReader::new(file));
        let mut reader = match dec.read_info() {
            Ok(r) => r,
            Err(e) => panic!("read_info failed: {e}"),
        };
        let Some(size) = reader.output_buffer_size() else {
            panic!("output_buffer_size unavailable")
        };
        let mut buf = vec![0; size];
        let info = match reader.next_frame(&mut buf) {
            Ok(i) => i,
            Err(e) => panic!("next_frame failed: {e}"),
        };
        let _ = std::fs::remove_file(&path); // ignore: 清理测试临时文件

        assert_eq!((info.width, info.height), (cw, ch));
        assert_eq!(info.buffer_size(), cw as usize * ch as usize * 3);
        eprintln!(
            "crops_the_captured_screen: captured {}x{} at ({}, {}), cropped {}x{}",
            w, h, shot.x, shot.y, cw, ch
        );
    }

    #[test]
    fn crop_png_reproduces_the_source_pixels() {
        let mut pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(4, 4);
        {
            let bytes = pixels.make_mut_bytes();
            for y in 0..4 {
                for x in 0..4 {
                    let idx = (y * 4 + x) * 4;
                    bytes[idx] = (x * 10) as u8;
                    bytes[idx + 1] = (y * 10) as u8;
                    bytes[idx + 2] = 255;
                    bytes[idx + 3] = 255;
                }
            }
        }
        let shot = Shot { x: 0, y: 0, pixels };
        let path =
            std::env::temp_dir().join(format!("tilex_pixel_test_{}.png", std::process::id()));
        if let Err(e) = super::crop_png(&shot, 1, 1, 2, 2, &path) {
            panic!("crop_png failed: {e}");
        }
        let file = match std::fs::File::open(&path) {
            Ok(f) => f,
            Err(e) => panic!("open failed: {e}"),
        };
        let dec = png::Decoder::new(std::io::BufReader::new(file));
        let mut reader = match dec.read_info() {
            Ok(r) => r,
            Err(e) => panic!("read_info failed: {e}"),
        };
        let Some(size) = reader.output_buffer_size() else {
            panic!("output_buffer_size unavailable")
        };
        let mut buf = vec![0; size];
        let info = match reader.next_frame(&mut buf) {
            Ok(i) => i,
            Err(e) => panic!("next_frame failed: {e}"),
        };
        let _ = std::fs::remove_file(&path); // ignore: 清理测试临时文件

        assert_eq!((info.width, info.height), (2, 2));
        assert_eq!(info.color_type, png::ColorType::Rgb);
        let expected = [
            10, 10, 255, 20, 10, 255, // row 0: (x=1,y=1), (x=2,y=1)
            10, 20, 255, 20, 20, 255, // row 1: (x=1,y=2), (x=2,y=2)
        ];
        assert_eq!(&buf[..info.buffer_size()], &expected);
    }

    #[test]
    fn rejects_a_selection_smaller_than_four_pixels() {
        let _serial = CACHE_DIR.lock();
        let pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(10, 10);
        let shot = Shot { x: 0, y: 0, pixels };
        let cache_dir = crate::platform::cache_dir();
        let before_files: Vec<_> = cache_dir
            .as_ref()
            .ok()
            .and_then(|d| std::fs::read_dir(d).ok())
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
            .unwrap_or_default();

        // 10px * 0.2 = 2px，小于 4px
        let result = super::crop_region(&shot, 0.0, 0.0, 0.2, 0.2);
        assert!(result.is_err());

        let after_files: Vec<_> = cache_dir
            .as_ref()
            .ok()
            .and_then(|d| std::fs::read_dir(d).ok())
            .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
            .unwrap_or_default();
        assert_eq!(before_files, after_files);
    }

    #[test]
    fn region_rect_adds_the_virtual_screen_origin() {
        let _serial = CACHE_DIR.lock();
        let pixels = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(100, 100);
        let shot = Shot {
            x: -1920,
            y: -200,
            pixels,
        };
        let region = match super::crop_region(&shot, 0.1, 0.1, 0.5, 0.5) {
            Ok(r) => r,
            Err(e) => panic!("crop_region failed: {e}"),
        };
        let _ = std::fs::remove_file(&region.path); // ignore: 清理测试临时文件

        assert_eq!(region.rect.l, -1910);
        assert_eq!(region.rect.t, -190);
        assert_eq!(region.rect.r, -1870);
        assert_eq!(region.rect.b, -150);
    }
}
