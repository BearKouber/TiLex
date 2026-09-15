//! Windows 开机自启（F11）：读写 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`。

use std::mem::size_of;

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
    let exe_str = exe.to_string_lossy();
    autostart_enabled_for_value(RUN_SUBKEY, VALUE_NAME, &exe_str)
}

/// 设置开机自启：开 = 写入完整 exe 路径（带引号）；关 = 删除键值（不存在不算错）。
pub fn set_autostart(on: bool) -> Result<(), Error> {
    let exe = std::env::current_exe()?;
    let exe_str = exe.to_string_lossy();
    set_autostart_for_value(RUN_SUBKEY, VALUE_NAME, &exe_str, on)
}

pub(super) fn autostart_enabled_for_value(
    subkey: PCWSTR,
    valuename: PCWSTR,
    target_exe: &str,
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
    let val = String::from_utf16_lossy(&buf[..len]);
    let clean = val.trim().trim_matches('"');
    Ok(clean.eq_ignore_ascii_case(target_exe))
}

pub(super) fn set_autostart_for_value(
    subkey: PCWSTR,
    valuename: PCWSTR,
    target_exe: &str,
    on: bool,
) -> Result<(), Error> {
    if on {
        let val = format!("\"{target_exe}\"");
        let wide: Vec<u16> = val.encode_utf16().chain(Some(0)).collect();
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

    #[test]
    #[ignore = "touches Windows registry"]
    fn test_autostart_registry_roundtrip() {
        let test_name = w!("TiLex_AutoStart_Test_Dummy");
        let fake_exe = "C:\\DummyPath\\TiLex_Test.exe";

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
