//! 微信 OCR 驱动与路径探测。
//!
//! 识别能力来自用户本机已安装的微信，通过独立进程 `tilex-ocr.exe` 与 `wcocr.dll` 进行驱动。

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use windows::Win32::Foundation::MAX_PATH;
use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};

use crate::error::Error;
use crate::platform::process;

/// 注册表 HKCU\Software\Tencent\Weixin 的 InstallPath。
/// 微信 4.x 装哪都行（本机可能在 D:\Weixin），不能写死路径。
fn install_path() -> Option<PathBuf> {
    let mut buf = [0u16; MAX_PATH as usize];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY: buf 容量为 MAX_PATH 宽字符，size 传入字节数；读取注册表字符串。
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            windows::core::w!(r"Software\Tencent\Weixin"),
            windows::core::w!("InstallPath"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    }
    .ok()
    .ok()?;
    // size 是字节数，且把结尾那个 NUL 也算进去了
    let len = (size as usize / 2).saturating_sub(1);
    // R-7：UTF-16 直接进 OsString，不经过 String —— `from_utf16_lossy` 会把装在
    // 非法代理对目录名里的微信替换成 U+FFFD，路径就再也打不开了。
    Some(PathBuf::from(OsString::from_wide(&buf[..len])))
}

/// 目录里挑版本号最大的子目录。版本号按分段数字比，不能按字符串比：
/// 字符串比会把 "8096" 排在 "812" 前面，也会把 "4.1.13.12" 排在 "4.1.9" 前面。
fn newest_child(dir: &Path, keep: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir() && keep(p))
        .max_by_key(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .split('.')
                .map(|s| s.parse::<u64>().unwrap_or(0))
                .collect::<Vec<_>>()
        })
}

/// 传给 wechat_ocr 的 wechat_dir：安装目录下的版本号子目录。
/// wcocr 用它定位 mmmojo_64.dll，并且会去启动 <wechat_dir>\..\weixin.exe。
fn wechat_dir() -> Option<PathBuf> {
    newest_child(&install_path()?, |p| p.join("mmmojo_64.dll").is_file())
}

/// 传给 wechat_ocr 的 ocr_exe：微信 4.x 的 OCR 引擎不在安装目录，在插件目录
/// %APPDATA%\Tencent\xwechat\XPlugin\Plugins\WeChatOcr\<版本>\extracted\wxocr.dll
fn ocr_engine() -> Option<PathBuf> {
    let appdata = std::env::var_os("APPDATA")?;
    let root = PathBuf::from(appdata).join("Tencent/xwechat/XPlugin/Plugins/WeChatOcr");
    let ver = newest_child(&root, |p| p.join("extracted").join("wxocr.dll").is_file())?;
    Some(ver.join("extracted").join("wxocr.dll"))
}

static WCOCR: OnceLock<PathBuf> = OnceLock::new();

/// wcocr.dll 作为外部文件随包分发，不编进主程序二进制。
/// 装机后在 $INSTDIR\vendor\ 或同目录；开发和 cargo test 时回落到源码树里的 vendor/。
fn wcocr_dll() -> Result<PathBuf, Error> {
    if let Some(cached) = WCOCR.get() {
        return Ok(cached.clone());
    }
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| Error::Platform("current_exe has no parent".into()))?;
    let candidates = [
        dir.join("vendor/wcocr.dll"),
        dir.join("wcocr.dll"),
        dir.join("../../../vendor/wcocr.dll"),
        dir.join("../../vendor/wcocr.dll"),
    ];
    let path = candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| Error::Platform(format!("wcocr.dll not found (near {})", dir.display())))?;
    let _ = WCOCR.set(path.clone()); // ignore: 并发设置相同路径，失败无所谓
    Ok(path)
}

/// tilex-ocr.exe 躺在主程序旁边：开发时在 target/release/，
/// cargo test 跑在 target/release/deps/，打包后在 exe 同级目录。
fn sidecar() -> Result<PathBuf, Error> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| Error::Platform("current_exe has no parent".into()))?;
    let candidates = [dir.join("tilex-ocr.exe"), dir.join("../tilex-ocr.exe")];
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| Error::Platform(format!("tilex-ocr.exe not found (near {})", dir.display())))
}

/// 微信 OCR 能不能用。能用返回微信版本号（04 的识别服务列表要显示），不能用返回缺什么。
pub fn wechat_ocr_status() -> Result<String, Error> {
    let dir = wechat_dir().ok_or_else(|| Error::Platform("WeChat is not installed".into()))?;
    ocr_engine().ok_or_else(|| {
        Error::Platform(
            "WeChat OCR engine (wxocr.dll) not found. Please log in to WeChat once to download the OCR plugin.".into(),
        )
    })?;
    Ok(dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned())
}

/// 校验 sidecar 的完整输出，成功时返回图里的文字（块按 top 升序，`\n` 连接）。
/// 先看输出再看退出码：拆解阶段失败时退出码非零，但结果已经是好的（旧规范「退出码 3 仍接受」）。
fn parse_output(stdout: &[u8], stderr: &[u8], exit_code: Option<i32>) -> Result<String, Error> {
    let invalid = |reason: &str| {
        Error::Platform(format!(
            "WeChat OCR: {reason} (exit={}, stdout-bytes={}, stderr-bytes={})",
            exit_code.map_or_else(|| "unknown".into(), |code| code.to_string()),
            stdout.len(),
            stderr.len(),
        ))
    };
    let text = std::str::from_utf8(stdout).map_err(|_| invalid("invalid UTF-8 output"))?;
    let json: serde_json::Value =
        serde_json::from_str(text).map_err(|_| invalid("invalid JSON output"))?;
    let code = json
        .get("errcode")
        .and_then(serde_json::Value::as_i64)
        .ok_or_else(|| invalid("invalid result status"))?;
    if code != 0 {
        return Err(Error::Platform(format!("WeChat OCR: errcode={code}")));
    }
    let blocks = json
        .get("ocr_response")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| invalid("invalid text blocks"))?;

    let mut valid_blocks: Vec<(&str, f64)> = Vec::new();
    for block in blocks {
        let value = block
            .get("text")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| invalid("invalid text field"))?;
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            let top = block
                .get("top")
                .and_then(serde_json::Value::as_f64)
                .unwrap_or(0.0);
            valid_blocks.push((value, top));
        }
    }

    // 图里没字不是错误：和 Umi 的 code 101 一样返回空串，由界面统一说「图里没有文字」。
    if valid_blocks.is_empty() {
        return Ok(String::new());
    }

    valid_blocks.sort_by(|a, b| a.1.total_cmp(&b.1));
    let result = valid_blocks
        .into_iter()
        .map(|(text, _)| text)
        .collect::<Vec<_>>()
        .join("\n");
    Ok(result)
}

/// 微信 OCR 识别一张图，返回图里的文字。几百毫秒到几秒，**不要在 UI 线程上调**（R-5）。
/// 没装微信、没下过 OCR 插件、识别不出字都返回 `Error::Platform`，消息是给用户看的。
pub fn wechat_ocr(image: &Path) -> Result<String, Error> {
    let engine = ocr_engine().ok_or_else(|| {
        Error::Platform(
            "WeChat OCR engine (wxocr.dll) not found. Please log in to WeChat once to download the OCR plugin.".into(),
        )
    })?;
    let runtime = wechat_dir().ok_or_else(|| Error::Platform("WeChat is not installed".into()))?;
    let sidecar = sidecar()?;
    let wcocr = wcocr_dll()?;

    let args: [OsString; 4] = [
        wcocr.into_os_string(),
        engine.into_os_string(),
        runtime.into_os_string(),
        image.as_os_str().to_owned(),
    ];

    let out = process::run(&sidecar, &args, Duration::from_secs(20))?;
    let result = parse_output(&out.stdout, &out.stderr, out.status.code());
    if result.is_ok() && !out.status.success() {
        log::warn!(
            "OCR: usable-output-after-exit status={} stdout-bytes={} stderr-bytes={}",
            out.status,
            out.stdout.len(),
            out.stderr.len()
        );
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err_msg(res: Result<String, Error>) -> String {
        match res {
            Err(Error::Platform(s)) => s,
            Err(e) => e.to_string(),
            Ok(s) => panic!("expected Err, got Ok({s})"),
        }
    }

    #[test]
    fn output_validation_precedes_exit_status_and_rejects_bad_shapes() {
        let success = br#"{"errcode":0,"ocr_response":[{"text":"recognized"}]}"#;
        for exit in [Some(0), Some(3), None] {
            let Ok(text) = super::parse_output(success, b"", exit) else {
                panic!("expected success for exit {exit:?}");
            };
            assert_eq!(text, "recognized");

            assert_eq!(
                err_msg(super::parse_output(br#"{"errcode":2}"#, b"", exit)),
                "WeChat OCR: errcode=2"
            );
            // 图里没字：空串，不是错误
            for empty in [
                r#"{"errcode":0,"ocr_response":[]}"#,
                r#"{"errcode":0,"ocr_response":[{"text":"  "}]}"#,
            ] {
                let Ok(text) = super::parse_output(empty.as_bytes(), b"", exit) else {
                    panic!("expected Ok for {empty}");
                };
                assert_eq!(text, "");
            }
            for bad in [
                "",
                "null",
                "[]",
                "{}",
                "{",
                "noise {\"errcode\":0}",
                "{\"errcode\":0,\"ocr_response\":{}}",
                "{\"errcode\":0,\"ocr_response\":[null]}",
                "{\"errcode\":0,\"ocr_response\":[{\"text\":3}]}",
                "{\"errcode\":0,\"ocr_response\":[{\"text\":\"ok\"},{}]}",
            ] {
                let error = err_msg(super::parse_output(
                    bad.as_bytes(),
                    b"sensitive diagnostic",
                    exit,
                ));
                assert!(!error.trim().is_empty());
                assert!(!error.contains("sensitive diagnostic"));
            }
            assert!(super::parse_output(&[0xff], b"", exit).is_err());
            let mut extra = success.to_vec();
            extra.extend_from_slice(b" trailing output");
            assert!(super::parse_output(&extra, b"", exit).is_err());
        }
    }

    #[test]
    fn sorts_blocks_by_top_and_filters_blank() {
        let json = r#"{
            "errcode": 0,
            "ocr_response": [
                {"top": 30, "text": "third"},
                {"top": 10, "text": "first"},
                {"text": "   "},
                {"top": 20, "text": "second"},
                {"top": "invalid", "text": "zero_top_str"},
                {"text": "zero_top_missing"}
            ]
        }"#;
        let Ok(result) = super::parse_output(json.as_bytes(), b"", Some(0)) else {
            panic!("parse_output failed");
        };
        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines.len(), 5);
        assert_eq!(lines[0], "zero_top_str");
        assert_eq!(lines[1], "zero_top_missing");
        assert_eq!(lines[2], "first");
        assert_eq!(lines[3], "second");
        assert_eq!(lines[4], "third");
    }

    #[test]
    #[allow(
        clippy::print_stderr,
        reason = "用于人工查看当前机器的微信路径探测结果"
    )]
    fn detects_wechat_paths() {
        eprintln!("install_path = {:?}", super::install_path());
        eprintln!("wechat_dir   = {:?}", super::wechat_dir());
        eprintln!("ocr_engine   = {:?}", super::ocr_engine());
    }

    #[test]
    #[allow(clippy::print_stderr, reason = "输出找到的 wcocr.dll 路径")]
    fn finds_wcocr_dll() {
        let Ok(path) = super::wcocr_dll() else {
            panic!("wcocr.dll not found");
        };
        eprintln!("wcocr_dll = {}", path.display());
    }

    #[test]
    #[ignore = "requires real environment and TILEX_OCR_TEST_IMAGE"]
    #[allow(clippy::print_stderr, reason = "输出端到端识别结果前缀")]
    fn wechat_ocr_reads_text() {
        let Ok(img) = std::env::var("TILEX_OCR_TEST_IMAGE") else {
            eprintln!("skipped: set TILEX_OCR_TEST_IMAGE to run");
            return;
        };
        let Ok(text) = super::wechat_ocr(Path::new(&img)) else {
            panic!("wechat_ocr failed");
        };
        // 按字符截，不按字节：识别出来的是中文，切在字节边界上会 panic。
        let head: String = text.chars().take(100).collect();
        eprintln!("recognized text:\n{head}");
        assert!(!text.trim().is_empty(), "recognized text is empty");
    }
}
