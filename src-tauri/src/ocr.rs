// 微信 OCR。识别能力全部来自用户本机已装的微信，我们只是驱动它：
//
//   vendor/wcocr.dll (167 KB)  ──mmmojo IPC──>  微信自带的 wxocr.dll + 模型
//                                               (24 MB，留在原地，不打包不复制)
//
// 没装微信就没有 OCR，这是明确的取舍：Windows 自带的 Windows.Media.Ocr 不用装东西，
// 但用户实测错误率比微信这套高不少。
//
// 真正调 DLL 的是 tilex-ocr.exe 那个独立进程（为什么见 ocr-sidecar/src/main.rs 顶部），
// 这里只负责：找到微信的两个路径 → 定位随包分发的 wcocr.dll → 起进程 → 收 JSON。

use once_cell::sync::OnceCell;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use windows::Win32::Foundation::MAX_PATH;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

/// 注册表 HKCU\Software\Tencent\Weixin 的 InstallPath。
/// 微信 4.x 装哪都行（本机在 D:\Weixin），不能写死路径。
fn install_path() -> Option<PathBuf> {
    let mut buf = [0u16; MAX_PATH as usize];
    let mut size = std::mem::size_of_val(&buf) as u32;
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            windows::core::w!(r"Software\Tencent\Weixin"),
            windows::core::w!("InstallPath"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr() as *mut _),
            Some(&mut size),
        )
    }
    .ok()
    .ok()?;
    // size 是字节数，且把结尾那个 NUL 也算进去了
    let len = (size as usize / 2).saturating_sub(1);
    Some(PathBuf::from(String::from_utf16_lossy(&buf[..len])))
}

/// 目录里挑版本号最大的子目录。版本号按分段数字比，不能按字符串比：
/// 字符串比会把 "8096" 排在 "812" 前面，也会把 "4.1.13.12" 排在 "4.1.9" 前面。
fn newest_child(dir: &Path, keep: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
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
    let root = dirs::config_dir()?.join("Tencent/xwechat/XPlugin/Plugins/WeChatOcr");
    let ver = newest_child(&root, |p| p.join("extracted/wxocr.dll").is_file())?;
    Some(ver.join("extracted/wxocr.dll"))
}

/// wcocr.dll 作为外部文件随包分发，不编进主程序二进制 —— 上游
/// swigger/wechat-ocr 没有 LICENSE，物理上隔开，主程序保持纯自有代码。
/// 打包时由 tauri.conf.json 的 bundle.resources 放到 exe 旁边。
fn wcocr_dll() -> Result<&'static Path, String> {
    static DLL: OnceCell<PathBuf> = OnceCell::new();
    DLL.get_or_try_init(|| {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let dir = exe.parent().ok_or("current_exe has no parent")?;
        // 装机后在 $INSTDIR\vendor\；开发和 cargo test 时回落到源码树里那一份。
        let candidates = [
            dir.join("vendor/wcocr.dll"),
            dir.join("wcocr.dll"),
            dir.join("../../../vendor/wcocr.dll"),
            dir.join("../../vendor/wcocr.dll"),
        ];
        candidates
            .into_iter()
            .find(|p| p.is_file())
            .ok_or_else(|| format!("找不到 wcocr.dll（在 {} 旁边）", dir.display()))
    })
    .map(|p| p.as_path())
}

/// tilex-ocr.exe 永远躺在主程序旁边：开发时是 workspace 共享的 target/release/，
/// 装机后是 tauri 的 externalBin 装到 $INSTDIR 的那一份（见 tauri.conf.json）。
fn sidecar() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("current_exe has no parent")?;
    // 上一层是给 cargo test 用的：测试跑在 target/release/deps/ 里，
    // 而 tilex-ocr.exe 在 target/release/。
    let candidates = [dir.join("tilex-ocr.exe"), dir.join("../tilex-ocr.exe")];
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| format!("找不到 tilex-ocr.exe（在 {} 旁边）", dir.display()))
}

/// 给设置界面看的：微信 OCR 到底能不能用。能用就返回微信版本号（也就是
/// wechat_dir 那个版本目录名），不能用就返回一句人话说明缺什么。
#[tauri::command(async)]
pub fn ocr_status() -> Result<String, String> {
    let dir = wechat_dir()
        .ok_or(r"没找到微信安装目录（注册表 HKCU\Software\Tencent\Weixin）")?;
    ocr_engine().ok_or(
        "没找到微信 OCR 引擎 wxocr.dll —— 装了微信还得登录过一次，它才会把 OCR 插件下下来",
    )?;
    Ok(dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned())
}

/// 识别一张图，返回 wcocr 原样吐出来的 JSON：
/// { errcode, imgpath, width, height, ocr_response: [{left,top,right,bottom,rate,text,details}] }
// Validate complete stdout before interpreting the exit: teardown may fail after
// a usable result. Diagnostics include lengths only, never image/text contents.
fn validate_output(stdout: &[u8], stderr: &[u8], exit_code: Option<i32>) -> Result<String, String> {
    let invalid = |reason: &str| format!(
        "WeChat OCR: {reason} (exit={}, stdout-bytes={}, stderr-bytes={})",
        exit_code.map(|code| code.to_string()).unwrap_or_else(|| "unknown".into()),
        stdout.len(), stderr.len(),
    );
    let text = std::str::from_utf8(stdout).map_err(|_| invalid("invalid UTF-8 output"))?;
    let json: serde_json::Value = serde_json::from_str(text).map_err(|_| invalid("invalid JSON output"))?;
    let code = json.get("errcode").and_then(serde_json::Value::as_i64)
        .ok_or_else(|| invalid("invalid result status"))?;
    if code != 0 { return Err(format!("WeChat OCR: errcode={code}")); }
    let blocks = json.get("ocr_response").and_then(serde_json::Value::as_array)
        .ok_or_else(|| invalid("invalid text blocks"))?;
    let mut has_text = false;
    for block in blocks {
        let value = block.get("text").and_then(serde_json::Value::as_str)
            .ok_or_else(|| invalid("invalid text field"))?;
        has_text |= !value.trim().is_empty();
    }
    if !has_text { return Err("OCR_NO_TEXT".into()); }
    Ok(text.trim().to_owned())
}

#[tauri::command(async)]
pub fn ocr_image(path: String) -> Result<String, String> {
    let engine = ocr_engine().ok_or("找不到微信 OCR 引擎（wxocr.dll）。需要先装微信并登录过一次。")?;
    let runtime = wechat_dir().ok_or(r"找不到微信安装目录（注册表 HKCU\Software\Tencent\Weixin）。")?;

    // CREATE_NO_WINDOW：tilex-ocr 是控制台程序，不挡的话每次识别都闪一个黑窗。
    let out = Command::new(sidecar()?)
        .creation_flags(0x08000000)
        .arg(wcocr_dll()?)
        .arg(&engine)
        .arg(&runtime)
        .arg(&path)
        .output()
        .map_err(|e| format!("起 tilex-ocr.exe 失败：{e}"))?;
    let result = validate_output(&out.stdout, &out.stderr, out.status.code());
    if result.is_ok() && !out.status.success() {
        log::warn!("OCR: usable-output-after-exit status={} stdout-bytes={} stderr-bytes={}",
            out.status, out.stdout.len(), out.stderr.len());
    }
    result
}

#[cfg(test)]
mod tests {
    #[test]
    fn output_validation_precedes_exit_status_and_rejects_bad_shapes() {
        let success = br#"{"errcode":0,"ocr_response":[{"text":"recognized"}]}"#;
        for exit in [Some(0), Some(3), None] {
            assert!(super::validate_output(success, b"", exit).is_ok());
            assert_eq!(super::validate_output(br#"{"errcode":2}"#, b"", exit).unwrap_err(), "WeChat OCR: errcode=2");
            for empty in [r#"{"errcode":0,"ocr_response":[]}"#, r#"{"errcode":0,"ocr_response":[{"text":"  "}]}"#] {
                assert_eq!(super::validate_output(empty.as_bytes(), b"", exit).unwrap_err(), "OCR_NO_TEXT");
            }
            for bad in ["", "null", "[]", "{}", "{", "noise {\"errcode\":0}",
                "{\"errcode\":0,\"ocr_response\":{}}", "{\"errcode\":0,\"ocr_response\":[null]}",
                "{\"errcode\":0,\"ocr_response\":[{\"text\":3}]}",
                "{\"errcode\":0,\"ocr_response\":[{\"text\":\"ok\"},{}]}"] {
                let error = super::validate_output(bad.as_bytes(), b"sensitive diagnostic", exit).unwrap_err();
                assert!(!error.trim().is_empty());
                assert!(!error.contains("sensitive diagnostic"));
            }
            assert!(super::validate_output(&[0xff], b"", exit).is_err());
            let mut extra = success.to_vec();
            extra.extend_from_slice(b" trailing output");
            assert!(super::validate_output(&extra, b"", exit).is_err());
        }
    }

    // 路径探测不碰 DLL，跑得飞快
    #[test]
    fn detects_wechat_paths() {
        eprintln!("install_path = {:?}", super::install_path());
        eprintln!("wechat_dir   = {:?}", super::wechat_dir());
        eprintln!("ocr_engine   = {:?}", super::ocr_engine());
    }

    // wcocr.dll 不再编进 exe，找不到就是打包漏了 bundle.resources，
    // 或者源码树里的 vendor/ 被误删 —— 这条必须红。
    #[test]
    fn finds_wcocr_dll() {
        let path = super::wcocr_dll().expect("wcocr.dll not found");
        eprintln!("wcocr_dll = {}", path.display());
    }

    // 端到端真的识别一次。要一张带字的图，用环境变量指过来：
    //   TILEX_OCR_TEST_IMAGE=D:/x/a.png cargo test --release ocr -- --nocapture
    // 没设就跳过 —— 没装微信的机器上不能让它红。
    //
    // 注意这里能在测试线程上跑，是因为真正调 DLL 的是独立进程的主线程；
    // 直接在这个线程上调 wechat_ocr 会永久挂住。
    #[test]
    fn wechat_ocr_reads_text() {
        let Ok(img) = std::env::var("TILEX_OCR_TEST_IMAGE") else {
            eprintln!("skipped: set TILEX_OCR_TEST_IMAGE to run");
            return;
        };
        let json = super::ocr_image(img).expect("ocr failed");
        eprintln!("{}", &json[..json.len().min(300)]);
        assert!(json.contains("\"errcode\":0"), "errcode not 0");
        assert!(json.contains("\"text\""), "no text block");
    }
}
