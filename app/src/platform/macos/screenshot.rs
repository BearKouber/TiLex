use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use super::super::process;
use crate::error::Error;

const SCREENCAPTURE: &str = "/usr/sbin/screencapture";

/// 系统自带的交互式区域截图：
/// - `-i`: 交互式选区；
/// - `-s`: 只允许拉框，禁掉按空格切「选窗口」模式（我们要的是一块矩形，不是一个窗口）；
/// - `-x`: 不播快门声。
pub fn pick_region_natively(out: &Path) -> Result<bool, Error> {
    let args = [
        OsString::from("-i"),
        OsString::from("-s"),
        OsString::from("-x"),
        out.as_os_str().to_os_string(),
    ];
    // 交互式选区等待用户操作，给 300 秒超时。
    process::run(Path::new(SCREENCAPTURE), &args, Duration::from_secs(300))?;

    // 判据是「文件非空」，不是退出码（用户取消时 screencapture 也可能返回 0 却什么都不写；
    // 若残留空文件也不算成功）。
    let ok = out.metadata().map(|m| m.len() > 0).unwrap_or(false);
    Ok(ok)
}
