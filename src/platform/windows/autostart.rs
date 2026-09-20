//! Windows 开机自启（F11）：读写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`。

use std::ffi::OsStr;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, w};

use crate::error::Error;

pub(super) const RUN_SUBKEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE_NAME: PCWSTR = w!("TiLex");

/// 检查开机自启是否开启：注册表中值存在且指向当前 exe（不区分大小写）才算开。
pub fn autostart_enabled() -> Result<bool, Error> {
    let exe = std::env::current_exe()?;
    autostart_enabled_for_value(RUN_SUBKEY, VALUE_NAME, exe.as_os_str())
}

/// 设置开机自启：开 = 写入完整 exe 路径（带引号）；关 = 删除键值（不存在不算错）。
pub fn set_autostart(on: bool) -> Result<(), Error> {
    let exe = std::env::current_exe()?;
    set_autostart_for_value(RUN_SUBKEY, VALUE_NAME, exe.as_os_str(), on)
}

/// UTF-16 层面的首尾修剪：空白和注册表值里那对引号。
fn trim_wide(mut s: &[u16]) -> &[u16] {
    const TRIM: [u16; 5] = [0x20, 0x09, 0x0D, 0x0A, b'"' as u16];
    while s.first().is_some_and(|c| TRIM.contains(c)) {
        s = &s[1..];
    }
    while s.last().is_some_and(|c| TRIM.contains(c)) {
        s = &s[..s.len() - 1];
    }
    s
}

/// 注册表里读出来的值是不是指向同一个 exe。
/// 全程走 UTF-16（R-7：路径不许经过 `to_string_lossy`），只对 ASCII 字母忽略大小写
/// —— Windows 路径大小写不敏感，非 ASCII 部分（`C:\Users\威泰普`）按码元原样比。
fn same_path(value: &[u16], target: &[u16]) -> bool {
    fn lower(c: u16) -> u16 {
        if (b'A' as u16..=b'Z' as u16).contains(&c) {
            c + 32
        } else {
            c
        }
    }
    let (a, b) = (trim_wide(value), trim_wide(target));
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| lower(*x) == lower(*y))
}

pub(super) fn autostart_enabled_for_value(
    subkey: PCWSTR,
    valuename: PCWSTR,
    target_exe: &OsStr,
) -> Result<bool, Error> {
    let mut size = 0u32;
    // SAFETY: subkey、valuename 为有效 PCWSTR；仅查询长度，数据缓冲区为空。
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey,
            valuename,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&raw mut size),
        )
    };
    if result != ERROR_SUCCESS || size == 0 {
        return Ok(false);
    }

    let mut buf = vec![0u16; (size as usize).div_ceil(2)];
    // SAFETY: buf 容量足够接收 size 字节；size 传入并在调用期间有效。
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey,
            valuename,
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&raw mut size),
        )
    };
    if result != ERROR_SUCCESS {
        return Ok(false);
    }

    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let target: Vec<u16> = target_exe.encode_wide().collect();
    Ok(same_path(&buf[..len], &target))
}

pub(super) fn set_autostart_for_value(
    subkey: PCWSTR,
    valuename: PCWSTR,
    target_exe: &OsStr,
    on: bool,
) -> Result<(), Error> {
    if on {
        const QUOTE: u16 = b'"' as u16;
        let wide: Vec<u16> = std::iter::once(QUOTE)
            .chain(target_exe.encode_wide())
            .chain([QUOTE, 0])
            .collect();
        let cbdata = (wide.len() * size_of::<u16>()) as u32;
        // SAFETY: wide 以 0 结尾；有效指针与长度。
        let result = unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                subkey,
                valuename,
                REG_SZ.0,
                Some(wide.as_ptr().cast()),
                cbdata,
            )
        };
        if result != ERROR_SUCCESS {
            return Err(Error::Platform(format!(
                "RegSetKeyValueW failed: {:?}",
                result
            )));
        }
    } else {
        // SAFETY: subkey 与 valuename 为有效 PCWSTR。
        let result = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey, valuename) };
        if result != ERROR_SUCCESS && result != ERROR_FILE_NOT_FOUND {
            return Err(Error::Platform(format!(
                "RegDeleteKeyValueW failed: {:?}",
                result
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s).encode_wide().collect()
    }

    #[test]
    fn same_path_ignores_quotes_whitespace_and_ascii_case() {
        let target = wide("C:\\Users\\威泰普\\TiLex\\tilex.exe");
        // 注册表里存的是带引号的形式，读回来要认得出是同一个
        assert!(same_path(
            &wide("\"C:\\Users\\威泰普\\TiLex\\tilex.exe\""),
            &target
        ));
        assert!(same_path(
            &wide("  c:\\users\\威泰普\\tilex\\TILEX.EXE  "),
            &target
        ));
        // 非 ASCII 部分按码元原样比：换一个字就不是同一个路径
        assert!(!same_path(
            &wide("C:\\Users\\威泰普x\\TiLex\\tilex.exe"),
            &target
        ));
        assert!(!same_path(
            &wide("C:\\Users\\威泰\\TiLex\\tilex.exe"),
            &target
        ));
        assert!(!same_path(&[], &target));
    }

    #[test]
    #[ignore = "touches Windows registry"]
    fn test_autostart_registry_roundtrip() {
        let test_name = w!("TiLex_AutoStart_Test_Dummy");
        let fake_exe = OsStr::new("C:\\DummyPath\\威泰普\\TiLex_Test.exe");

        // 清理旧残留
        let _ = set_autostart_for_value(RUN_SUBKEY, test_name, fake_exe, false); // ignore: cleanup before test

        // 初始读 -> false
        let enabled = autostart_enabled_for_value(RUN_SUBKEY, test_name, fake_exe).unwrap();
        assert!(!enabled);

        // 写 -> true
        set_autostart_for_value(RUN_SUBKEY, test_name, fake_exe, true).unwrap();
        let enabled = autostart_enabled_for_value(RUN_SUBKEY, test_name, fake_exe).unwrap();
        assert!(enabled);

        // 删 -> false
        set_autostart_for_value(RUN_SUBKEY, test_name, fake_exe, false).unwrap();
        let enabled = autostart_enabled_for_value(RUN_SUBKEY, test_name, fake_exe).unwrap();
        assert!(!enabled);
    }
}
