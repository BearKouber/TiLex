use std::ffi::OsString;
use std::io::Write;
use std::path::Path;

use super::super::process;
use crate::error::Error;

pub fn copy_text(text: &str) -> Result<(), Error> {
    let mut child = process::spawn_piped(Path::new("/usr/bin/pbcopy"), &[])?;
    if let Some(mut stdin) = child.stdin.take() {
        if let Err(e) = stdin.write_all(text.as_bytes()) {
            let _ = child.kill(); // ignore: 写入失败回收子进程
            let _ = child.wait(); // ignore: 避免僵尸进程
            return Err(e.into());
        }
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Platform(format!("pbcopy exited with {status}")))
    }
}

// 不加 `--` 隔开参数：`open` 自己的 `--args` 是「后面的都传给被打开的程序」，
// 单独的 `--` 在它这里没验证过。调用方给的是数据目录 / 日志目录 / https 链接，
// 都不会以 `-` 开头，加了反而可能白白弄坏。
pub fn open_path(path: &Path) -> Result<(), Error> {
    process::spawn(
        Path::new("/usr/bin/open"),
        &[path.as_os_str().to_os_string()],
    )
}

pub fn open_url(url: &str) -> Result<(), Error> {
    process::spawn(Path::new("/usr/bin/open"), &[OsString::from(url)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires macOS GUI session and real clipboard"]
    fn real_copy_text() {
        // 写进去还要用 pbpaste 读回来比对：只断言 Ok 等于没测。
        let marker = "tilex 剪贴板往返 ✓";
        copy_text(marker).unwrap();
        let out = process::run(
            Path::new("/usr/bin/pbpaste"),
            &[],
            std::time::Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), marker);
    }
}
