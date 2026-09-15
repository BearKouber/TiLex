use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::Media::Speech::{
    ISpObjectTokenCategory, ISpVoice, SPCAT_VOICES, SPF_ASYNC, SPF_IS_NOT_XML,
    SPF_PURGEBEFORESPEAK, SpObjectTokenCategory, SpVoice,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Threading::WaitForSingleObject;
use windows::core::{PCWSTR, w};

use super::super::Voice;
use crate::error::Error;

static SPEAK_GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn speak(text: &str, voice: Voice, done: impl FnOnce() + Send + 'static) -> Result<(), Error> {
    if text.trim().is_empty() {
        done();
        return Ok(());
    }

    let mut text_wide: Vec<u16> = OsStr::new(text).encode_wide().collect();
    text_wide.push(0);

    let generation = SPEAK_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;

    thread::Builder::new()
        .name("tts".into())
        .spawn(move || {
            let result = run_tts(&text_wide, voice, generation);
            if let Err(e) = result {
                log::error!("TTS COM error: {e}");
            }
            done();
        })
        .map_err(|e| Error::Platform(format!("spawn tts thread: {e}")))?;

    Ok(())
}

pub fn stop_speaking() {
    SPEAK_GENERATION.fetch_add(1, Ordering::SeqCst);
}

fn run_tts(text_wide: &[u16], voice: Voice, generation: u64) -> Result<(), windows::core::Error> {
    // SAFETY: 本线程是刚起的朗读线程，初始化成功才配对 CoUninitialize。
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok()?;

    let result = run_tts_inner(text_wide, voice, generation);

    // SAFETY: Uninitializes COM on this thread.
    unsafe {
        CoUninitialize();
    }

    result
}

fn run_tts_inner(
    text_wide: &[u16],
    voice: Voice,
    generation: u64,
) -> Result<(), windows::core::Error> {
    // SAFETY: SpVoice is a valid COM class. Created on an initialized thread.
    let sp_voice: ISpVoice = unsafe { CoCreateInstance(&SpVoice, None, CLSCTX_ALL) }?;

    // SAFETY: SpObjectTokenCategory is a valid COM class.
    let category: ISpObjectTokenCategory =
        unsafe { CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_ALL) }?;

    // SAFETY: SPCAT_VOICES is a valid registry path. No raw pointers are retained.
    unsafe { category.SetId(SPCAT_VOICES, false)? };

    let lang_id = match voice {
        Voice::Chinese => w!("Language=804"),
        Voice::English => w!("Language=409"),
    };

    // SAFETY: lang_id is a valid PCWSTR. EnumTokens returns the enumerated tokens.
    if let Ok(tokens) = unsafe { category.EnumTokens(lang_id, PCWSTR::null()) } {
        let mut count = 0;
        // SAFETY: Calling GetCount on valid IEnumSpObjectTokens.
        if unsafe { tokens.GetCount(&mut count) }.is_ok() && count > 0 {
            // SAFETY: Retrieving the first token.
            if let Ok(token) = unsafe { tokens.Item(0) } {
                // SAFETY: Setting voice with valid ISpObjectToken.
                if let Err(e) = unsafe { sp_voice.SetVoice(&token) } {
                    log::warn!("TTS: SetVoice failed, using the default voice: {e}");
                }
            }
        } else {
            warn_fallback(voice);
        }
    } else {
        warn_fallback(voice);
    }

    // SAFETY: Passing valid null-terminated wide string to Speak. Using SPF_IS_NOT_XML to avoid XML parsing errors.
    unsafe {
        sp_voice.Speak(
            PCWSTR(text_wide.as_ptr()),
            (SPF_ASYNC.0 | SPF_IS_NOT_XML.0 | SPF_PURGEBEFORESPEAK.0) as u32,
            None,
        )?;
    }

    // 不用 WaitUntilDone：它超时返回 S_FALSE，windows-rs 的 `.ok()` 把 S_FALSE 也当成功，读 50ms 就会被当成读完。
    // SAFETY: sp_voice 有效；返回的事件句柄归语音对象所有，不关闭，sp_voice 活着时一直有效。
    let finished = unsafe { sp_voice.SpeakCompleteEvent() };
    loop {
        if SPEAK_GENERATION.load(Ordering::SeqCst) != generation {
            // 被 stop_speaking 或新的 speak 打断。
            // SAFETY: 空文字 + PURGEBEFORESPEAK 是 SAPI 规定的打断写法。
            unsafe {
                let _ = sp_voice.Speak(PCWSTR::null(), SPF_PURGEBEFORESPEAK.0 as u32, None); // ignore: 打断失败也马上释放语音对象，声音随之停止
            }
            return Ok(());
        }
        // SAFETY: finished 是上面拿到的有效事件句柄。
        match unsafe { WaitForSingleObject(finished, 50) } {
            WAIT_OBJECT_0 => return Ok(()),
            WAIT_TIMEOUT => {}
            _ => return Err(windows::core::Error::from_win32()),
        }
    }
}

fn warn_fallback(voice: Voice) {
    let name = match voice {
        Voice::Chinese => "Chinese",
        Voice::English => "English",
    };
    log::warn!("TTS: no voice found for language {}", name);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    // 要有播放设备：没有时 SAPI 的 Speak 报 SPERR_NOT_FOUND（0x8004503A），done 立刻回来，下面的耗时断言失败。
    #[test]
    #[ignore]
    fn speaks_both_voices() {
        let started = std::time::Instant::now();
        let (tx, rx) = mpsc::channel();

        let tx1 = tx.clone();
        speak("测试中文发音", Voice::Chinese, move || {
            tx1.send(()).unwrap()
        })
        .unwrap();
        rx.recv_timeout(Duration::from_secs(30)).unwrap();

        let tx2 = tx.clone();
        speak("Testing English voice", Voice::English, move || {
            tx2.send(()).unwrap()
        })
        .unwrap();
        rx.recv_timeout(Duration::from_secs(30)).unwrap();
        // 两句真读完要一两秒；出错时 done 立刻回来。
        assert!(
            started.elapsed() > Duration::from_millis(800),
            "no audio played"
        );
    }

    #[test]
    fn speak_empty_string() {
        let (tx, rx) = mpsc::channel();
        speak("   ", Voice::Chinese, move || tx.send(()).unwrap()).unwrap();
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
    }
}
