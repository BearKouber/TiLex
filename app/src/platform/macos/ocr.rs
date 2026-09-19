use std::path::{Path, PathBuf};
use std::time::Duration;

use super::super::process;
use crate::error::Error;

/// tilex-ocr 躺在主程序旁边：开发时在 target/release/，
/// cargo test 跑在 target/release/deps/，打包后在 exe 同级目录。
fn sidecar() -> Result<PathBuf, Error> {
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| Error::Platform("current_exe has no parent".into()))?;
    let candidates = [dir.join("tilex-ocr"), dir.join("../tilex-ocr")];
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| Error::Platform(format!("tilex-ocr not found (near {})", dir.display())))
}

/// Apple Vision 识别一张图，返回图里的文字。几百毫秒到几秒，**不要在 UI 线程上调**（R-5）。
pub fn apple_ocr(image: &Path) -> Result<String, Error> {
    let sidecar_path = sidecar()?;
    let args = [image.as_os_str().to_os_string()];
    let out = process::run(&sidecar_path, &args, Duration::from_secs(30))?;
    if !out.status.success() {
        let err_msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(Error::Platform(if err_msg.is_empty() {
            format!("tilex-ocr exited with status: {}", out.status)
        } else {
            err_msg
        }));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Apple Vision 能不能用。能用返回引擎名（识别服务列表要显示），不能用返回缺什么。
pub fn apple_ocr_status() -> Result<String, Error> {
    sidecar()?;
    Ok("Apple Vision".into())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    #[ignore = "requires macOS with the Swift sidecar built"]
    #[allow(clippy::print_stderr, reason = "测试跳过或打印诊断输出")]
    fn live_apple_ocr_icon() {
        if sidecar().is_err() {
            eprintln!("skipped: tilex-ocr sidecar not found");
            return;
        }
        // icon.png 是「文 / A」两个字形，一张图同时验证三件事：sidecar 跑得起来、
        // Vision 调通了、recognitionLanguages 里的中文真的生效（只留默认的 en-US
        // 时「文」认不出来，这个断言就会挂）。
        let icon = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("icon.png");
        let Ok(text) = apple_ocr(&icon) else {
            panic!("apple_ocr failed on icon.png");
        };
        assert!(text.contains('文'), "中文没认出来: {text:?}");
        assert!(text.contains('A'), "英文没认出来: {text:?}");
    }

    #[test]
    #[ignore = "requires macOS with the Swift sidecar built"]
    #[allow(clippy::print_stderr, reason = "测试跳过或打印诊断输出")]
    fn live_apple_ocr_status() {
        if sidecar().is_err() {
            eprintln!("skipped: tilex-ocr sidecar not found");
            return;
        }
        let Ok(engine) = apple_ocr_status() else {
            panic!("apple_ocr_status failed");
        };
        assert_eq!(engine, "Apple Vision");
    }
}
