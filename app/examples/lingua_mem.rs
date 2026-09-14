//! D13 语种识别内存实测，不进产物。
//!
//!   cargo run --release --example lingua_mem --features detect-full -- <high|low> <none|detect|unload>
//!
//! none = 只建检测器（基线）；detect = 识别一个单词和两句话；unload = 识别后 `unload_language_models()`。
//! 做完打印 `ready`，然后睡 10 分钟，外面用 PowerShell 读专用工作集后结束进程。
#![allow(clippy::print_stdout, reason = "实测工具，输出给人看")]

use std::time::{Duration, Instant};

use lingua::LanguageDetectorBuilder;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let low = args.first().is_some_and(|a| a == "low");
    let stage = args.get(1).map_or("detect", String::as_str);

    let mut builder = LanguageDetectorBuilder::from_all_languages();
    if low {
        builder.with_low_accuracy_mode();
    }
    let detector = builder.build();

    if stage != "none" {
        let started = Instant::now();
        // 法语那句用来确认模型真的加载了（lite 没有法语，认成别的或 None 是对的）。
        for text in [
            "unprecedented",
            "He decided to abandon the plan after the meeting.",
            "Il a décidé d'abandonner le projet après la réunion.",
        ] {
            let lang = detector.detect_language_of(text);
            println!("{text:?} -> {lang:?}");
        }
        println!("detect took {} ms", started.elapsed().as_millis());
    }
    if stage == "unload" {
        detector.unload_language_models();
    }
    println!("ready");
    std::thread::sleep(Duration::from_secs(600));
}
