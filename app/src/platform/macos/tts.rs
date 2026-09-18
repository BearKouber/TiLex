use std::ffi::OsString;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use super::super::Voice;
use super::super::process;
use crate::error::Error;

// 这两个名字要等朋友真机手测确认（语音没装时 say 会非零退出），到时候可能要改。
const VOICE_CHINESE: &str = "Tingting";
const VOICE_ENGLISH: &str = "Samantha";

static CURRENT_CHILD: Mutex<Option<std::process::Child>> = Mutex::new(None);
static SPEAK_GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn speak(text: &str, voice: Voice, done: impl FnOnce() + Send + 'static) -> Result<(), Error> {
    // 空文本先返回，不打断正在读的那次（和 Windows 的 tts.rs 一致）。
    if text.trim().is_empty() {
        done();
        return Ok(());
    }

    stop_speaking();

    let voice_name = match voice {
        Voice::Chinese => VOICE_CHINESE,
        Voice::English => VOICE_ENGLISH,
    };
    let args = [
        OsString::from("-v"),
        OsString::from(voice_name),
        OsString::from("--"),
        OsString::from(text),
    ];

    let mut child = process::spawn_piped(Path::new("/usr/bin/say"), &args)?;
    let _ = child.stdin.take(); // ignore: 关闭 stdin 给 say 发送 EOF

    let generation = SPEAK_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    *CURRENT_CHILD.lock().unwrap_or_else(PoisonError::into_inner) = Some(child);

    thread::Builder::new()
        .name("tts".into())
        .spawn(move || {
            let status = loop {
                thread::sleep(Duration::from_millis(20));
                if SPEAK_GENERATION.load(Ordering::SeqCst) != generation {
                    break None;
                }
                let mut guard = CURRENT_CHILD.lock().unwrap_or_else(PoisonError::into_inner);
                if SPEAK_GENERATION.load(Ordering::SeqCst) != generation {
                    break None;
                }
                let Some(child) = guard.as_mut() else {
                    break None;
                };
                match child.try_wait() {
                    Ok(Some(status)) => {
                        let _ = guard.take(); // ignore: 朗读结束，清理子进程句柄
                        break Some(status);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        log::error!("TTS: try_wait failed: {e}");
                        let _ = guard.take(); // ignore: 异常退出，清理子进程句柄
                        break None;
                    }
                }
            };

            if let Some(status) = status
                && !status.success()
            {
                log::error!("TTS: say exited with non-zero status: {status}");
            }

            // 无条件调用：契约是「读完、被停止或出错后恰好一次」（platform/mod.rs），
            // 被新一次朗读顶掉也算「被停止」。调用方自己有代次（pop_result.rs 的 SPEAK_TOKEN），
            // 过期的 done 不会动到 UI。少调一次反而会让按钮永远停在「停止」。
            done();
        })
        .map_err(|e| {
            let mut guard = CURRENT_CHILD.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(mut child) = guard.take() {
                let _ = child.kill(); // ignore: 线程启动失败，回收子进程
                let _ = child.wait(); // ignore: 回收避免僵尸进程
            }
            Error::Platform(format!("spawn tts thread: {e}"))
        })?;

    Ok(())
}

pub fn stop_speaking() {
    let mut guard = CURRENT_CHILD.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(mut child) = guard.take() {
        let _ = child.kill(); // ignore: 进程可能已自行退出，kill 失败无所谓
        let _ = child.wait(); // ignore: 回收子进程避免僵尸进程
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_returns_ok_and_calls_done() {
        // done 是 'static 的，不能借局部变量。
        let called = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&called);
        let res = speak("   ", Voice::Chinese, move || {
            flag.store(true, Ordering::SeqCst);
        });
        assert!(res.is_ok(), "{res:?}");
        assert!(called.load(Ordering::SeqCst));
    }

    #[test]
    #[ignore = "requires macOS real hardware to produce audio output"]
    fn real_say_speaks() {
        let (tx, rx) = std::sync::mpsc::channel();
        let res = speak("hello", Voice::English, move || {
            let _ = tx.send(()); // ignore: 测试 channel
        });
        assert!(res.is_ok(), "{res:?}");
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(()));
    }
}
