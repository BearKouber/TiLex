//! 增强选中识别（`selection.force_copy`）。有的程序自己画文字，UIA 读不到选区
//! （微信 4.x 聊天气泡就是起因）。开关打开、且两条 UIA 链上都没有元素**报出空选区**时，
//! 模拟一次 Ctrl+C 读出文字，再把原来的剪贴板写回去。契约见 platform-windows.md。
//!
//! 文字当 UIA 答案交给状态机，不当剪贴板候选：恢复会改剪贴板序号，剪贴板那条路复核序号时会把浮标立刻收回。
//! 只在取词 worker 上跑。日志只记大小和耗时，不记内容。
//! 本文件的 `open` / `read_text` 也是剪贴板兜底读文字的唯一实现（design §3：不引 arboard）。

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardFormatNameW, GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::core::w;

/// 模拟 Ctrl+C 有害、也拿不到文字的程序。匹配规则同用户黑名单；只影响模拟复制，不影响浮标。
/// 终端：Ctrl+C 是中断。多数终端有 UIA 走不到这里，但 mintty 之类在鼠标上报模式（vim、htop）下
/// 既没有选区也没有 TextPattern。
/// 资源管理器：Ctrl+C 复制的是选中的文件，永远不是文字，还会在剪贴板历史里留一条（双击桌面会走到这里）。
pub const NO_FORCE_COPY: &str = "mintty, alacritty, wezterm-gui, putty, kitty, conemu, conemu64, tabby, \
    hyper, windowsterminal, openconsole, conhost, cmd, powershell, pwsh, wsl, mobaxterm, xshell, \
    securecrt, termius, explorer";

const CF_UNICODETEXT: u32 = 13;
const MAX_BYTES: usize = 32 << 20;
const STEP: Duration = Duration::from_millis(10);
/// 小于 CLIP_GRACE_MS（600），目标程序晚写的复制还能走剪贴板那条路。
const COPY_WAIT: Duration = Duration::from_millis(500);
/// 等目标程序把其他格式写完再打开剪贴板。
const SETTLE: Duration = Duration::from_millis(30);

pub struct Inputs {
    pub enabled: bool,
    pub saw_empty_selection: bool,
    pub clipboard_changed: bool,
    pub modifiers_down: bool,
    /// 前台进程在 NO_FORCE_COPY 里
    pub excluded_app: bool,
}

pub fn eligible(i: &Inputs) -> bool {
    // 报出了空选区 = 真的没选中（Word / 普通网页里随手拖一下），不许发 Ctrl+C。
    // 注意不是「见过 TextPattern」：浏览器内置 PDF 阅读器有 TextPattern 但报不了选区，
    // 那种要放行，否则增强选中识别在 PDF 里等于没有（P13）。
    // 剪贴板变了 = 程序选中即复制，监听那边会处理。
    i.enabled
        && !i.saw_empty_selection
        && !i.clipboard_changed
        && !i.modifiers_down
        && !i.excluded_app
}

/// GDI 句柄不是 HGLOBAL，不能按字节拷（CF_BITMAP 会由系统从 CF_DIB 补回）；私有格式带着程序自己的句柄；
/// OLE 格式指向原主人的对象；三个标记格式由 `write` 自己写，跳过。
pub fn restorable(format: u32, name: &str) -> bool {
    const SKIP: [&str; 5] = [
        "DataObject",
        "Ole Private Data",
        "ExcludeClipboardContentFromMonitorProcessing",
        "CanIncludeInClipboardHistory",
        "CanUploadToCloudClipboard",
    ];
    !matches!(
        format,
        2 | 3 | 9 | 14 | 0x80 | 0x82 | 0x83 | 0x8E | 0x200..=0x3FF
    ) && !SKIP.iter().any(|s| s.eq_ignore_ascii_case(name))
}

/// 只在我们自己的复制之上恢复。任何别的变化都是别人更新的内容。
pub fn should_restore(now: u32, copied: u32) -> bool {
    now == copied
}

pub fn modifiers_down() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
        .iter()
        // SAFETY: 无指针参数。
        .any(|vk| unsafe { GetAsyncKeyState(i32::from(vk.0)) } < 0)
}

/// 备份 → Ctrl+C → 等序号变化 → 读文字 → 写回备份。读剪贴板数据会让 OLE 原主人补写延迟渲染的格式，
/// 所以两个序号都在关闭剪贴板之后读。`still_current` 在按键发出前再查一次：原主人渲染的格式多时，备份本身就要一会儿。
pub fn copy_selection(still_current: impl Fn() -> bool) -> String {
    let started = Instant::now();
    let Some(snapshot) = Snapshot::take() else {
        return String::new();
    };
    // SAFETY: 无参数。
    let before = unsafe { GetClipboardSequenceNumber() };
    if !still_current() {
        return String::new();
    }
    if !send_ctrl_c() {
        log::debug!("PopButton: force copy input was blocked");
        return String::new();
    }
    let deadline = Instant::now() + COPY_WAIT;
    loop {
        std::thread::sleep(STEP);
        // SAFETY: 无参数。
        if unsafe { GetClipboardSequenceNumber() } != before {
            break;
        }
        if Instant::now() >= deadline {
            log::debug!("PopButton: force copy saw no clipboard change");
            return String::new();
        }
    }
    std::thread::sleep(SETTLE);
    let Some(text) = read_text() else {
        return String::new();
    };
    // SAFETY: 无参数。
    let copied = unsafe { GetClipboardSequenceNumber() };
    let restored = snapshot.restore(copied);
    log::info!(
        "PopButton: force copy read {} chars, {} formats / {} bytes backed up, restored {}, {} ms",
        text.chars().count(),
        snapshot.formats.len(),
        snapshot.formats.iter().map(|(_, b)| b.len()).sum::<usize>(),
        restored,
        started.elapsed().as_millis(),
    );
    text
}

pub struct Snapshot {
    formats: Vec<(u32, Vec<u8>)>,
}

impl Snapshot {
    /// 剪贴板忙或超过 MAX_BYTES 时返回 None。空剪贴板也是有效快照：恢复它就是清空。
    pub fn take() -> Option<Snapshot> {
        let _open = open()?;
        let mut formats = Vec::new();
        let mut total = 0usize;
        let mut format = 0;
        loop {
            // SAFETY: 剪贴板由 _open 打开着。
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                break;
            }
            if !restorable(format, &format_name(format)) {
                continue;
            }
            // SAFETY: 同上；返回的句柄在剪贴板关闭前有效。
            let Ok(handle) = (unsafe { GetClipboardData(format) }) else {
                continue;
            };
            // SAFETY: restorable 已排除非 HGLOBAL 的格式。
            total += unsafe { GlobalSize(HGLOBAL(handle.0)) };
            if total > MAX_BYTES {
                log::info!("PopButton: force copy skipped, clipboard over 32MB");
                return None;
            }
            // SAFETY: 同上。
            if let Some(bytes) = unsafe { locked(handle) } {
                formats.push((format, bytes));
            }
        }
        Some(Snapshot { formats })
    }

    /// 比对和写入都在剪贴板打开期间做，中间没人能插进新内容。
    pub fn restore(&self, copied: u32) -> bool {
        let Some(_open) = open() else {
            return false;
        };
        // SAFETY: 无参数。
        should_restore(unsafe { GetClipboardSequenceNumber() }, copied) && write(&self.formats)
    }
}

/// 剪贴板打开期间的守卫，drop 时关闭。
struct Open;

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: 只在 open() 成功后构造。
        let _ = unsafe { CloseClipboard() }; // ignore: 关不上时下一次 OpenClipboard 会失败并记日志
    }
}

/// 别的程序写剪贴板时会占着它一小会儿，重试 10 次。
fn open() -> Option<Open> {
    for _ in 0..10 {
        // SAFETY: 不关联窗口。
        if unsafe { OpenClipboard(None) }.is_ok() {
            return Some(Open);
        }
        std::thread::sleep(STEP);
    }
    log::warn!("PopButton: could not open the clipboard");
    None
}

/// 读剪贴板里的文字（去掉首尾空白）。只有文件或图片、没有 CF_UNICODETEXT 时是空串：不是文字选区。
/// 打不开剪贴板返回 None。
pub fn read_text() -> Option<String> {
    let _open = open()?;
    // SAFETY: 剪贴板打开着；CF_UNICODETEXT 是 HGLOBAL。
    let bytes = unsafe { GetClipboardData(CF_UNICODETEXT) }
        .ok()
        .and_then(|h| unsafe { locked(h) })
        .unwrap_or_default();
    let wide: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&c| u16::from_ne_bytes(c))
        .take_while(|&c| c != 0)
        .collect();
    Some(String::from_utf16_lossy(&wide).trim().to_string())
}

fn format_name(format: u32) -> String {
    if format < 0xC000 {
        return String::new();
    }
    let mut buf = [0u16; 256];
    // SAFETY: buf 在调用期间有效，长度由切片给出。
    let len = unsafe { GetClipboardFormatNameW(format, &mut buf) }.max(0) as usize;
    String::from_utf16_lossy(&buf[..len])
}

/// # Safety
/// `handle` 必须是剪贴板打开期间取到的 HGLOBAL。
unsafe fn locked(handle: HANDLE) -> Option<Vec<u8>> {
    let mem = HGLOBAL(handle.0);
    // SAFETY: 调用方保证 mem 是有效的 HGLOBAL；锁住期间按 GlobalSize 读，读完解锁。
    unsafe {
        let ptr = GlobalLock(mem) as *const u8;
        if ptr.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(ptr, GlobalSize(mem)).to_vec();
        let _ = GlobalUnlock(mem); // ignore: 返回值是"还锁着吗"，不是错误
        Some(bytes)
    }
}

/// 剪贴板必须已经打开。三个标记让写回的内容不进 Win+V 历史、云剪贴板和剪贴板管理器：它已经被记过一次了。
fn write(formats: &[(u32, Vec<u8>)]) -> bool {
    // SAFETY: 调用方已打开剪贴板。
    if unsafe { EmptyClipboard() }.is_err() {
        return false;
    }
    for (format, bytes) in formats {
        set(*format, bytes);
    }
    for marker in [
        w!("ExcludeClipboardContentFromMonitorProcessing"),
        w!("CanIncludeInClipboardHistory"),
        w!("CanUploadToCloudClipboard"),
    ] {
        // SAFETY: 常量宽字符串。
        set(
            unsafe { RegisterClipboardFormatW(marker) },
            &0u32.to_ne_bytes(),
        );
    }
    true
}

/// 写进剪贴板成功返回 true。恢复旧内容、打标记时失败只能认，不看返回值；用户主动复制要看（`write_text`）。
fn set(format: u32, bytes: &[u8]) -> bool {
    // SAFETY: 新分配的内存按长度拷贝；SetClipboardData 成功后内存归系统，失败还是我们的，自己释放。
    unsafe {
        let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) else {
            return false;
        };
        let ptr = GlobalLock(mem) as *mut u8;
        if ptr.is_null() {
            let _ = GlobalFree(mem); // ignore: 释放失败只是漏这一块
            return false;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
        let _ = GlobalUnlock(mem); // ignore: 返回值是"还锁着吗"，不是错误
        if SetClipboardData(format, HANDLE(mem.0)).is_err() {
            let _ = GlobalFree(mem); // ignore: 释放失败只是漏这一块
            return false;
        }
        true
    }
}

/// 将文本写入系统剪贴板（CF_UNICODETEXT），不加排除历史标记（用户主动复制进入历史）。
pub(crate) fn write_text(text: &str) -> bool {
    let Some(_open) = open() else {
        return false;
    };
    // SAFETY: open() 成功保证剪贴板已打开。
    if unsafe { EmptyClipboard() }.is_err() {
        return false;
    }
    let bytes: Vec<u8> = text
        .encode_utf16()
        .chain([0])
        .flat_map(|c| c.to_ne_bytes())
        .collect();
    // 剪贴板已清空，写失败要报出去，不然复制按钮亮绿勾、剪贴板却是空的
    set(CF_UNICODETEXT, &bytes)
}

fn send_ctrl_c() -> bool {
    let key = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                dwFlags: flags,
                ..Default::default()
            },
        },
    };
    let c = VIRTUAL_KEY(u16::from(b'C'));
    let down = KEYBD_EVENT_FLAGS(0);
    let inputs = [
        key(VK_CONTROL, down),
        key(c, down),
        key(c, KEYEVENTF_KEYUP),
        key(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    // UIPI 会拦下发给提权窗口的输入：发出 0 个，等同于没选中。
    // SAFETY: inputs 在调用期间有效，结构体大小按类型给出。
    unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) == inputs.len() as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// OS 剪贴板只有一份，下面两条 live 测试都要独占它一整段（写入→读序号→恢复）。
    /// 并发跑会互相把序号顶掉，随机失败。串起来跑，`--test-threads` 给多少都无所谓。
    /// 中毒了照样往下走：上一条测试 panic 不该把这条也变成失败。
    static CLIPBOARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock_clipboard() -> std::sync::MutexGuard<'static, ()> {
        CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn inputs() -> Inputs {
        Inputs {
            enabled: true,
            saw_empty_selection: false,
            clipboard_changed: false,
            modifiers_down: false,
            excluded_app: false,
        }
    }

    #[test]
    fn copies_only_when_every_condition_holds() {
        assert!(eligible(&inputs()));
        for blocked in [
            Inputs {
                enabled: false,
                ..inputs()
            },
            Inputs {
                saw_empty_selection: true,
                ..inputs()
            },
            Inputs {
                clipboard_changed: true,
                ..inputs()
            },
            Inputs {
                modifiers_down: true,
                ..inputs()
            },
            Inputs {
                excluded_app: true,
                ..inputs()
            },
        ] {
            assert!(!eligible(&blocked));
        }
    }

    #[test]
    fn skips_gdi_private_and_ole_formats() {
        for format in [
            2, 3, 9, 14, 0x80, 0x82, 0x83, 0x8E, 0x200, 0x2FF, 0x300, 0x3FF,
        ] {
            assert!(!restorable(format, ""), "{format:#x}");
        }
        for name in [
            "DataObject",
            "Ole Private Data",
            "CanIncludeInClipboardHistory",
        ] {
            assert!(!restorable(0xC100, name), "{name}");
        }
        // CF_TEXT、CF_DIB、CF_UNICODETEXT、CF_HDROP、CF_DIBV5、注册的 HTML 格式
        for (format, name) in [
            (1, ""),
            (8, ""),
            (13, ""),
            (15, ""),
            (17, ""),
            (0xC100, "HTML Format"),
        ] {
            assert!(restorable(format, name), "{format:#x} {name}");
        }
    }

    #[test]
    fn restores_only_over_our_own_copy() {
        assert!(should_restore(7, 7));
        assert!(!should_restore(8, 7));
        assert!(!should_restore(0, u32::MAX));
    }

    /// 写入并返回关闭剪贴板之后的序号（和 copy_selection 读的方式一样）。
    fn put(formats: &[(u32, Vec<u8>)]) -> u32 {
        {
            let _open = open().expect("open clipboard");
            assert!(write(formats));
        }
        unsafe { GetClipboardSequenceNumber() }
    }

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16()
            .chain([0])
            .flat_map(|c| c.to_ne_bytes())
            .collect()
    }

    // 用本机真实剪贴板，跑完恢复原内容：cargo test --release force_copy -- --ignored
    #[test]
    #[ignore]
    fn clipboard_round_trip() {
        let _guard = lock_clipboard();
        let original = Snapshot::take().expect("take the user's clipboard");
        let custom = unsafe { RegisterClipboardFormatW(w!("TiLex Force Copy Test")) };
        put(&[
            (CF_UNICODETEXT, utf16("before 划词")),
            (custom, vec![1, 2, 3, 0, 255]),
        ]);
        let before = Snapshot::take().unwrap();
        assert!(
            before
                .formats
                .iter()
                .any(|(f, b)| *f == custom && b == &[1, 2, 3, 0, 255])
        );

        // 我们的复制落地、没人再写：逐格式恢复。
        let copied = put(&[(CF_UNICODETEXT, utf16("selection"))]);
        assert_eq!(read_text().unwrap(), "selection");
        assert_eq!(
            unsafe { GetClipboardSequenceNumber() },
            copied,
            "reading must not move the sequence"
        );
        assert!(before.restore(copied));
        let after = Snapshot::take().unwrap();
        for format in &before.formats {
            assert!(
                after.formats.contains(format),
                "format {:#x} not restored",
                format.0
            );
        }
        assert_eq!(read_text().unwrap(), "before 划词");

        // 有人在我们之后复制：保留他的内容。
        let copied = put(&[(CF_UNICODETEXT, utf16("selection"))]);
        put(&[(CF_UNICODETEXT, utf16("user copy"))]);
        assert!(!before.restore(copied));
        assert_eq!(read_text().unwrap(), "user copy");

        let now = unsafe { GetClipboardSequenceNumber() };
        assert!(original.restore(now));
    }

    #[test]
    #[ignore]
    fn write_text_round_trip() {
        let _guard = lock_clipboard();
        let original = Snapshot::take().expect("take the user's clipboard");
        let sample = "Hello 世界 🚀 TiLex 测试";
        assert!(write_text(sample));
        assert_eq!(read_text().as_deref(), Some(sample));
        let now = unsafe { GetClipboardSequenceNumber() };
        assert!(original.restore(now));
    }
}
