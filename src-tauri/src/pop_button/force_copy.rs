// Enhanced selection (pop_button_force_copy, off by default). Some apps draw
// their own text and expose no selection through UIA - WeChat 4.x chat bubbles
// are the case that prompted this. With the switch on, and only when no element
// on either UIA chain has a TextPattern, we synthesise one Ctrl+C, read the
// text, then put the previous clipboard back.
//
// The text is handed to the state machine as a UIA answer, not as a clipboard
// candidate: restoring changes the clipboard sequence, and the clipboard path
// would take the button straight back down when it rechecks the sequence.
// Runs on the worker only. Logs carry sizes and timings, never content.

use log::{debug, info, warn};
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData,
    GetClipboardFormatNameW, GetClipboardSequenceNumber, OpenClipboard, RegisterClipboardFormatW,
    SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};

// Apps where a synthetic Ctrl+C does harm and never yields text. Same matching
// rules as the user blacklist; only affects the copy, not the button.
// Terminals: Ctrl+C is an interrupt. Most expose UIA and never get this far, but
// mintty and friends in mouse-reporting mode (vim, htop) show no selection and
// no TextPattern.
// Explorer: Ctrl+C copies the selected files, never text, and still leaves an
// entry in clipboard history (desktop double-clicks land here).
pub const NO_FORCE_COPY: &str = "mintty, alacritty, wezterm-gui, putty, kitty, conemu, conemu64, tabby, \
    hyper, windowsterminal, openconsole, conhost, cmd, powershell, pwsh, wsl, mobaxterm, xshell, \
    securecrt, termius, explorer";

const CF_UNICODETEXT: u32 = 13;
const MAX_BYTES: usize = 32 << 20;
const STEP: Duration = Duration::from_millis(10);
// Below CLIP_GRACE_MS (600), so a late copy still reaches the clipboard path.
const COPY_WAIT: Duration = Duration::from_millis(500);
// Let the app finish writing its other formats before we open the clipboard.
const SETTLE: Duration = Duration::from_millis(30);

pub struct Inputs {
    pub enabled: bool,
    pub saw_text_pattern: bool,
    pub clipboard_changed: bool,
    pub modifiers_down: bool,
    pub excluded_app: bool, // foreground process is in NO_FORCE_COPY
}

pub fn eligible(i: &Inputs) -> bool {
    // A TextPattern with an empty selection means nothing is selected. A
    // clipboard change means the app copied on select; the listener has it.
    i.enabled && !i.saw_text_pattern && !i.clipboard_changed && !i.modifiers_down && !i.excluded_app
}

// GDI handles are not HGLOBALs and cannot be copied as bytes; CF_BITMAP comes
// back on its own from CF_DIB. Private formats carry app-owned handles of any
// kind. OLE formats point into the old owner's objects. The three markers are
// skipped because restore() always writes them itself.
pub fn restorable(format: u32, name: &str) -> bool {
    const SKIP: [&str; 5] = [
        "DataObject",
        "Ole Private Data",
        "ExcludeClipboardContentFromMonitorProcessing",
        "CanIncludeInClipboardHistory",
        "CanUploadToCloudClipboard",
    ];
    !matches!(format, 2 | 3 | 9 | 14 | 0x80 | 0x82 | 0x83 | 0x8E | 0x200..=0x3FF)
        && !SKIP.iter().any(|s| s.eq_ignore_ascii_case(name))
}

// Only restore over our own copy. Any other change is someone's newer content.
pub fn should_restore(now: u32, copied: u32) -> bool {
    now == copied
}

pub fn modifiers_down() -> bool {
    [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
        .iter()
        .any(|vk| unsafe { GetAsyncKeyState(vk.0 as i32) } < 0)
}

// Backup -> Ctrl+C -> wait for the sequence to move -> read the text -> put
// the backup back. Reading clipboard data can make an OLE owner render
// delayed formats, so both sequence numbers are taken after the clipboard is
// closed again. `still_current` is checked right before the keys go out: the
// backup itself can take a while when the old owner renders a lot.
pub fn copy_selection(still_current: impl Fn() -> bool) -> String {
    let started = Instant::now();
    let Some(snapshot) = Snapshot::take() else { return String::new() };
    let before = unsafe { GetClipboardSequenceNumber() };
    if !still_current() {
        return String::new();
    }
    if !send_ctrl_c() {
        debug!("PopButton: force copy input was blocked");
        return String::new();
    }
    let deadline = Instant::now() + COPY_WAIT;
    loop {
        std::thread::sleep(STEP);
        if unsafe { GetClipboardSequenceNumber() } != before {
            break;
        }
        if Instant::now() >= deadline {
            debug!("PopButton: force copy saw no clipboard change");
            return String::new();
        }
    }
    std::thread::sleep(SETTLE);
    let Some(text) = read_text() else { return String::new() };
    let copied = unsafe { GetClipboardSequenceNumber() };
    let restored = snapshot.restore(copied);
    info!(
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
    // None when the clipboard is busy or holds more than MAX_BYTES. An empty
    // clipboard is a valid snapshot: restoring it empties the clipboard.
    pub fn take() -> Option<Snapshot> {
        let _open = open()?;
        unsafe {
            let mut formats = Vec::new();
            let mut total = 0usize;
            let mut format = 0;
            loop {
                format = EnumClipboardFormats(format);
                if format == 0 {
                    break;
                }
                if !restorable(format, &format_name(format)) {
                    continue;
                }
                let Ok(handle) = GetClipboardData(format) else { continue };
                total += GlobalSize(HGLOBAL(handle.0));
                if total > MAX_BYTES {
                    info!("PopButton: force copy skipped, clipboard over 32MB");
                    return None;
                }
                if let Some(bytes) = locked(handle) {
                    formats.push((format, bytes));
                }
            }
            Some(Snapshot { formats })
        }
    }

    // Comparing and writing both happen with the clipboard open, so nobody
    // can slip new content in between.
    pub fn restore(&self, copied: u32) -> bool {
        let Some(_open) = open() else { return false };
        unsafe { should_restore(GetClipboardSequenceNumber(), copied) && write(&self.formats) }
    }
}

struct Open;

impl Drop for Open {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

// Other programs hold the clipboard open for a moment while they write.
fn open() -> Option<Open> {
    for _ in 0..10 {
        if unsafe { OpenClipboard(None) }.is_ok() {
            return Some(Open);
        }
        std::thread::sleep(STEP);
    }
    warn!("PopButton: force copy could not open the clipboard");
    None
}

// Files or images without CF_UNICODETEXT read as empty: not a text selection.
fn read_text() -> Option<String> {
    let _open = open()?;
    let bytes = unsafe { GetClipboardData(CF_UNICODETEXT).ok().and_then(|h| locked(h)) }.unwrap_or_default();
    let wide: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_ne_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    Some(String::from_utf16_lossy(&wide).trim().to_string())
}

unsafe fn format_name(format: u32) -> String {
    if format < 0xC000 {
        return String::new();
    }
    let mut buf = [0u16; 256];
    let len = GetClipboardFormatNameW(format, &mut buf).max(0) as usize;
    String::from_utf16_lossy(&buf[..len])
}

unsafe fn locked(handle: HANDLE) -> Option<Vec<u8>> {
    let mem = HGLOBAL(handle.0);
    let ptr = GlobalLock(mem) as *const u8;
    if ptr.is_null() {
        return None;
    }
    let bytes = std::slice::from_raw_parts(ptr, GlobalSize(mem)).to_vec();
    let _ = GlobalUnlock(mem);
    Some(bytes)
}

// Clipboard must be open. The markers keep the restored copy out of Win+V
// history, cloud clipboard and clipboard managers: it was recorded once already.
unsafe fn write(formats: &[(u32, Vec<u8>)]) -> bool {
    if EmptyClipboard().is_err() {
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
        set(RegisterClipboardFormatW(marker), &0u32.to_ne_bytes());
    }
    true
}

unsafe fn set(format: u32, bytes: &[u8]) {
    let Ok(mem) = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) else { return };
    let ptr = GlobalLock(mem) as *mut u8;
    if ptr.is_null() {
        let _ = GlobalFree(mem);
        return;
    }
    std::ptr::copy_nonoverlapping(bytes.as_ptr(), ptr, bytes.len());
    let _ = GlobalUnlock(mem);
    // On success the system owns the memory; on failure it is still ours.
    if SetClipboardData(format, HANDLE(mem.0)).is_err() {
        let _ = GlobalFree(mem);
    }
}

fn send_ctrl_c() -> bool {
    let key = |vk: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, dwFlags: flags, ..Default::default() },
        },
    };
    let c = VIRTUAL_KEY(b'C' as u16);
    let down = KEYBD_EVENT_FLAGS(0);
    let inputs = [
        key(VK_CONTROL, down),
        key(c, down),
        key(c, KEYEVENTF_KEYUP),
        key(VK_CONTROL, KEYEVENTF_KEYUP),
    ];
    // UIPI blocks input to elevated windows: 0 events sent, same as no selection.
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) == inputs.len() as u32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> Inputs {
        Inputs {
            enabled: true,
            saw_text_pattern: false,
            clipboard_changed: false,
            modifiers_down: false,
            excluded_app: false,
        }
    }

    #[test]
    fn copies_only_when_every_condition_holds() {
        assert!(eligible(&inputs()));
        for blocked in [
            Inputs { enabled: false, ..inputs() },
            Inputs { saw_text_pattern: true, ..inputs() },
            Inputs { clipboard_changed: true, ..inputs() },
            Inputs { modifiers_down: true, ..inputs() },
            Inputs { excluded_app: true, ..inputs() },
        ] {
            assert!(!eligible(&blocked));
        }
    }

    #[test]
    fn skips_gdi_private_and_ole_formats() {
        for format in [2, 3, 9, 14, 0x80, 0x82, 0x83, 0x8E, 0x200, 0x2FF, 0x300, 0x3FF] {
            assert!(!restorable(format, ""), "{format:#x}");
        }
        for name in ["DataObject", "Ole Private Data", "CanIncludeInClipboardHistory"] {
            assert!(!restorable(0xC100, name), "{name}");
        }
        // CF_TEXT, CF_DIB, CF_UNICODETEXT, CF_HDROP, CF_DIBV5, registered HTML
        for (format, name) in [(1, ""), (8, ""), (13, ""), (15, ""), (17, ""), (0xC100, "HTML Format")] {
            assert!(restorable(format, name), "{format:#x} {name}");
        }
    }

    #[test]
    fn restores_only_over_our_own_copy() {
        assert!(should_restore(7, 7));
        assert!(!should_restore(8, 7));
        assert!(!should_restore(0, u32::MAX));
    }

    // Returns the sequence after closing, the way copy_selection reads it.
    unsafe fn put(formats: &[(u32, Vec<u8>)]) -> u32 {
        {
            let _open = open().expect("open clipboard");
            assert!(write(formats));
        }
        GetClipboardSequenceNumber()
    }

    fn utf16(s: &str) -> Vec<u8> {
        s.encode_utf16().chain([0]).flat_map(|c| c.to_ne_bytes()).collect()
    }

    // Uses the real clipboard of this desktop, and puts it back afterwards.
    // cargo test --release force_copy -- --ignored
    #[test]
    #[ignore]
    fn clipboard_round_trip() {
        let original = Snapshot::take().expect("take the user's clipboard");
        unsafe {
            let custom = RegisterClipboardFormatW(w!("TiLex Force Copy Test"));
            put(&[(CF_UNICODETEXT, utf16("before 划词")), (custom, vec![1, 2, 3, 0, 255])]);
            let before = Snapshot::take().unwrap();
            assert!(before.formats.iter().any(|(f, b)| *f == custom && b == &[1, 2, 3, 0, 255]));

            // Our copy lands, nothing else writes: restored, format for format.
            let copied = put(&[(CF_UNICODETEXT, utf16("selection"))]);
            assert_eq!(read_text().unwrap(), "selection");
            assert_eq!(GetClipboardSequenceNumber(), copied, "reading must not move the sequence");
            assert!(before.restore(copied));
            let after = Snapshot::take().unwrap();
            for format in &before.formats {
                assert!(after.formats.contains(format), "format {:#x} not restored", format.0);
            }
            assert_eq!(read_text().unwrap(), "before 划词");

            // Someone copies after us: their content stays.
            let copied = put(&[(CF_UNICODETEXT, utf16("selection"))]);
            put(&[(CF_UNICODETEXT, utf16("user copy"))]);
            assert!(!before.restore(copied));
            assert_eq!(read_text().unwrap(), "user copy");
        }
        let now = unsafe { GetClipboardSequenceNumber() };
        assert!(original.restore(now));
    }
}
