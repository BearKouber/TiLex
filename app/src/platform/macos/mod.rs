use std::fs::{self, OpenOptions, TryLockError};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use super::process;
use crate::error::Error;

mod proxy;
pub use proxy::system_proxy;

const LOCK_FILE: &str = "tilex.lock";
const SOCKET: &str = "tilex.sock";

pub fn data_dir() -> Result<PathBuf, Error> {
    let home = std::env::var_os("HOME").ok_or_else(|| Error::Platform("HOME is not set".into()))?;
    Ok(PathBuf::from(home).join("Library/Application Support/TiLex"))
}

/// `flock` 锁文件判定谁是第一个；第二个连一下第一个的 Unix socket 就算通知。
pub fn claim_single_instance(wait: Duration) -> Result<bool, Error> {
    let dir = data_dir()?;
    fs::create_dir_all(&dir)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK_FILE))?;
    let deadline = Instant::now() + wait;
    loop {
        match file.try_lock() {
            Ok(()) => {
                // 锁跟着文件描述符走：不关文件，进程退出时系统释放。
                std::mem::forget(file);
                return Ok(true);
            }
            Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(TryLockError::WouldBlock) => break,
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
    }
    // 连不上（老实例的 socket 还没建好）就只是这次没打开设置，照样退出。
    let _ = UnixStream::connect(dir.join(SOCKET)); // ignore: 见上一行
    Ok(false)
}

pub fn listen_activation(on_activate: impl Fn() + Send + 'static) -> Result<(), Error> {
    let path = data_dir()?.join(SOCKET);
    // 上次异常退出会留下旧 socket 文件，bind 前删掉；拿到锁说明没有别的实例在用它。
    let _ = fs::remove_file(&path); // ignore: 文件不存在是正常情况
    let listener = UnixListener::bind(&path)?;
    std::thread::Builder::new()
        .name("instance-listener".into())
        .spawn(move || {
            for connection in listener.incoming() {
                match connection {
                    Ok(_) => on_activate(),
                    Err(e) => {
                        // 不重试：持续失败时循环会刷爆日志；少一次"打开设置"无伤大雅。
                        log::error!("Instance: accept failed, stop listening: {e}");
                        return;
                    }
                }
            }
        })?;
    Ok(())
}

pub fn open_path(path: &Path) -> Result<(), Error> {
    let out = process::run(
        Path::new("/usr/bin/open"),
        &[path.into()],
        Duration::from_secs(10),
    )?;
    if out.status.success() {
        Ok(())
    } else {
        Err(Error::Platform(format!("open exited with {}", out.status)))
    }
}

/// macOS 的无边框窗口圆角由 AppKit 负责，这里不做事。
pub fn round_corners(_window: &slint::Window) -> Result<(), Error> {
    Ok(())
}

// ponytail: B6 用 NSApp activate 实现；现在调用方只记日志，设置窗口照常打开。
pub fn bring_to_front(_window: &slint::Window) -> Result<(), Error> {
    Err(Error::Unsupported)
}
