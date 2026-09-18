//! 子进程。全项目唯一出现 `Command::new` 的文件（design R-4），std 跨平台实现。

use std::ffi::OsString;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::error::Error;

/// 运行子进程，收集 stdout / stderr。超过 `timeout`（含读完输出）就杀掉并返回 `Error::Timeout`。
/// 不判断退出码：有的程序（微信 OCR）输出完有效结果后以非零退出，由调用方先校验输出。
pub fn run(program: &Path, args: &[OsString], timeout: Duration) -> Result<Output, Error> {
    let mut child = command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // 两个管道各一个线程读：只读一个时，子进程写满另一个就会和我们互相等（管道死锁）。
    let stdout = drain(child.stdout.take());
    let stderr = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()?
            && stdout.is_finished()
            && stderr.is_finished()
        {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill(); // ignore: 子进程可能刚好自己退出了，kill 失败无所谓
            let _ = child.wait(); // ignore: 只为回收进程不留僵尸；读线程在管道关闭后自己结束
            return Err(Error::Timeout);
        }
        thread::sleep(Duration::from_millis(10));
    };
    Ok(Output {
        status,
        stdout: join(stdout),
        stderr: join(stderr),
    })
}

/// 启动一个不等待、不接管道的子进程（托盘"重启"拉起新实例用）。
pub fn spawn(program: &Path, args: &[OsString]) -> Result<(), Error> {
    command(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

/// 启动子进程，stdin 接管道交给调用方（写完 drop 就是 EOF），stdout/stderr 丢弃。
/// 调用方自己负责 `wait()` 或 `kill()`。
#[cfg(target_os = "macos")]
pub fn spawn_piped(program: &Path, args: &[OsString]) -> Result<std::process::Child, Error> {
    let child = command(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(child)
}

/// release 版是 GUI 程序、没有控制台：Windows 上起控制台子进程（tilex-ocr.exe 等）
/// 每次都会闪一个黑窗，要 CREATE_NO_WINDOW（旧版 ocr.rs 同款）。
fn command(program: &Path) -> Command {
    #[allow(unused_mut, reason = "只有 Windows 分支会改它")]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn drain(pipe: Option<impl Read + Send + 'static>) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.read_to_end(&mut buf); // ignore: 读错就当输出到此为止，调用方校验内容
        }
        buf
    })
}

fn join(handle: JoinHandle<Vec<u8>>) -> Vec<u8> {
    // 只在 is_finished() 之后调用，不会阻塞；读线程里没有会 panic 的代码。
    handle.join().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(script: &str) -> (&'static Path, Vec<OsString>) {
        if cfg!(windows) {
            (Path::new("cmd"), vec!["/C".into(), script.into()])
        } else {
            (Path::new("sh"), vec!["-c".into(), script.into()])
        }
    }

    #[test]
    fn collects_output_and_does_not_judge_exit_code() {
        let (program, args) = shell("echo hello&& exit 3");
        let out = run(program, &args, Duration::from_secs(10)).unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains("hello"));
        assert_eq!(out.status.code(), Some(3));
    }

    #[test]
    fn timeout_kills_the_child() {
        let (program, args) = if cfg!(windows) {
            (
                Path::new("ping"),
                vec!["-n".into(), "30".into(), "127.0.0.1".into()],
            )
        } else {
            (Path::new("sleep"), vec!["30".into()])
        };
        let started = Instant::now();
        let result = run(program, &args, Duration::from_millis(300));
        assert!(matches!(result, Err(Error::Timeout)), "{result:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn missing_program_is_an_error() {
        let result = run(
            Path::new("tilex-no-such-program"),
            &[],
            Duration::from_secs(5),
        );
        assert!(matches!(result, Err(Error::Io(_))), "{result:?}");
    }
}
