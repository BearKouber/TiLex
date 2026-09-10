// 微信 OCR 的执行者，独立进程。参数：<wcocr.dll> <wxocr.dll> <微信版本目录> <图片>
// 成功时把 wcocr 原样吐的 JSON 打到 stdout，失败时错误信息打到 stderr 并非零退出。
//
// 为什么要单独一个进程，而不是在主程序里直接调：
//
//  1. **必须在主线程调**。实测在 std::thread::spawn 出来的线程上调 wechat_ocr
//     会永久挂住（不是超时，是不返回），主线程上则 0.5 秒出结果。原因没深究，
//     多半是 mmmojo 的 IPC 绑在首次调用的线程上。独立进程的 main() 天然就是主线程，
//     省得去赌 tauri 把某个 command 派到哪个线程上。
//  2. **崩溃隔离**。真正干活的是微信那个 24 MB 的闭源 wxocr.dll，
//     它崩了只会带走这个进程，不会把主程序一起带走。
//
// 一次完整调用（握手 + 两次识别）实测 0.54 秒，所以不用常驻，一次一进程就够。

use std::ffi::{c_char, c_void, CStr, CString};
use std::os::windows::ffi::OsStrExt;
use std::sync::OnceLock;

#[link(name = "kernel32")]
extern "system" {
    fn LoadLibraryW(name: *const u16) -> *mut c_void;
    fn GetProcAddress(module: *mut c_void, name: *const u8) -> *const c_void;
}

type WechatOcrFn = unsafe extern "C" fn(
    ocr_exe: *const u16,
    wechat_dir: *const u16,
    imgfn: *const c_char,
    set_res: Option<extern "C" fn(*const c_char)>,
) -> bool;

// 回调是个裸函数指针，没有 context 参数，结果只能往全局塞。
// OnceLock 而不是 static mut：后者在 2024 edition 里取引用是硬错误。
static RESULT: OnceLock<String> = OnceLock::new();

extern "C" fn on_result(json: *const c_char) {
    if json.is_null() {
        return;
    }
    let text = unsafe { CStr::from_ptr(json) }.to_string_lossy().into_owned();
    let _ = RESULT.set(text);
}

fn wide(s: &str) -> Vec<u16> {
    std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 5 {
        die("usage: tilex-ocr <wcocr.dll> <wxocr.dll> <wechat_version_dir> <image>");
    }
    let (dll, engine, wechat_dir, image) = (&args[1], &args[2], &args[3], &args[4]);

    // LoadLibraryW / ocr_exe / wechat_dir 都收宽字符，所以中文用户名的路径没问题。
    // 只有 imgfn 是窄字符 const char*，实测传 UTF-8 的中文路径也能过（对面原样回显）。
    let ocr: WechatOcrFn = unsafe {
        let module = LoadLibraryW(wide(dll).as_ptr());
        if module.is_null() {
            die(&format!("LoadLibraryW failed: {dll}"));
        }
        let proc = GetProcAddress(module, b"wechat_ocr\0".as_ptr());
        if proc.is_null() {
            die("wcocr.dll has no export named wechat_ocr");
        }
        std::mem::transmute(proc)
    };

    let img = match CString::new(image.as_str()) {
        Ok(v) => v,
        Err(_) => die("image path contains a NUL byte"),
    };
    let ok = unsafe {
        ocr(
            wide(engine).as_ptr(),
            wide(wechat_dir).as_ptr(),
            img.as_ptr(),
            Some(on_result),
        )
    };
    if !ok {
        die("wechat_ocr returned false (连不上微信 OCR，微信没装或版本对不上)");
    }
    let json = match RESULT.get() {
        Some(json) => json,
        None => die("wechat_ocr succeeded but produced no result"),
    };
    println!("{json}");

    // 收尾：不显式 stop_ocr 的话，进程退出时 wcocr 里那个全局 g_instance 的析构
    // 会在已经拆掉的 IPC 上崩掉 —— 实测退出码 3，而 stdout 里结果是好的。
    // 收完了就直接 exit(0)，不让 CRT 再去跑那堆析构。
    unsafe {
        let module = LoadLibraryW(wide(dll).as_ptr());
        let proc = GetProcAddress(module, b"stop_ocr\0".as_ptr());
        if !proc.is_null() {
            let stop: unsafe extern "C" fn() = std::mem::transmute(proc);
            stop();
        }
    }
    use std::io::Write;
    let _ = std::io::stdout().flush();
    std::process::exit(0);
}
