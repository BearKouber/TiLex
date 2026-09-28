use windows::Win32::Foundation::LPARAM;
use windows::Win32::Graphics::Gdi::{
    DEFAULT_CHARSET, EnumFontFamiliesExW, GetDC, LOGFONTW, RASTER_FONTTYPE, ReleaseDC, TEXTMETRICW,
};

/// 装在系统里的字体家族名，去重、排序。名字是 GDI 给的（中文系统下中文字体是中文名），
/// Slint 底下的 fontique 把 DirectWrite 的所有本地化名都登记了，两种名字都认。
pub fn font_families() -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let lf = LOGFONTW {
        lfCharSet: DEFAULT_CHARSET,
        ..Default::default()
    };
    // SAFETY: 回调只在 EnumFontFamiliesExW 执行期间被同步调用，lparam 指向的 names 活得更久。
    unsafe {
        let dc = GetDC(None);
        EnumFontFamiliesExW(
            dc,
            &lf,
            Some(collect),
            LPARAM(&mut names as *mut Vec<String> as isize),
            0,
        );
        ReleaseDC(None, dc);
    }
    names.sort();
    names.dedup();
    names
}

unsafe extern "system" fn collect(
    lf: *const LOGFONTW,
    _tm: *const TEXTMETRICW,
    font_type: u32,
    lparam: LPARAM,
) -> i32 {
    // SAFETY: GDI 保证 lf 在回调期间有效；lparam 是上面传进来的 &mut Vec<String>。
    let (lf, names) = unsafe { (&*lf, &mut *(lparam.0 as *mut Vec<String>)) };
    let len = lf
        .lfFaceName
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(lf.lfFaceName.len());
    let name = String::from_utf16_lossy(&lf.lfFaceName[..len]);
    // 位图字体 Slint 画不了；@ 开头的是竖排版本
    if font_type & RASTER_FONTTYPE != 0 || name.is_empty() || name.starts_with('@') {
        return 1;
    }
    names.push(name);
    1
}
