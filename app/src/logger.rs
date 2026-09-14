//! 文件日志：`<数据目录>/logs/tilex.log`，超过 1 MB 轮换一次到 `tilex.old.log`。panic 也写进来。
//! 规矩（旧规范沿用）：模块前缀 `Config: ...`；热路径不记；不记 API key、原文、剪贴板、截图、完整 stdout。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use log::{LevelFilter, Log, Metadata, Record};

use crate::error::Error;

const MAX_BYTES: u64 = 1024 * 1024;
const FILE: &str = "tilex.log";
const OLD_FILE: &str = "tilex.old.log";

static LOGGER: OnceLock<FileLog> = OnceLock::new();

struct FileLog {
    dir: PathBuf,
    /// 打开的文件和它当前的大小。轮换后重开失败就是 None，之后的日志丢掉（没地方报告）。
    file: Mutex<Option<(File, u64)>>,
}

/// 初始化日志并装上 panic hook。一个进程只能调一次。
pub fn init(data_dir: &Path) -> Result<(), Error> {
    let dir = data_dir.join("logs");
    fs::create_dir_all(&dir)?;
    let file = open(&dir)?;
    let size = file.metadata()?.len();
    LOGGER
        .set(FileLog {
            dir,
            file: Mutex::new(Some((file, size))),
        })
        .map_err(|_| Error::Platform("logger already initialized".into()))?;
    let Some(logger) = LOGGER.get() else {
        return Err(Error::Platform("logger missing after set".into()));
    };
    log::set_logger(logger).map_err(|e| Error::Platform(e.to_string()))?;
    log::set_max_level(LevelFilter::Info);

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!(
            "panic: {info}\n{}",
            std::backtrace::Backtrace::force_capture()
        );
        previous(info);
    }));
    Ok(())
}

/// 日志目录（托盘"查看日志"打开它）。没初始化时是 None。
pub fn dir() -> Option<&'static Path> {
    LOGGER.get().map(|l| l.dir.as_path())
}

fn open(dir: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(FILE))
}

impl Log for FileLog {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!("{} {:<5} {}\n", now(), record.level(), record.args());
        let mut slot = self.file.lock().unwrap_or_else(PoisonError::into_inner);
        if slot
            .as_ref()
            .is_some_and(|(_, size)| size + line.len() as u64 > MAX_BYTES)
        {
            *slot = None; // 先关掉再改名
            let _ = fs::rename(self.dir.join(FILE), self.dir.join(OLD_FILE)); // ignore: 改名失败就接着往原文件追加，下次再试
            *slot = open(&self.dir).ok().map(|f| {
                let size = f.metadata().map_or(0, |m| m.len());
                (f, size)
            });
        }
        if let Some((file, size)) = slot.as_mut()
            && file.write_all(line.as_bytes()).is_ok()
        {
            *size += line.len() as u64;
        }
    }

    fn flush(&self) {}
}

/// `2026-09-14 07:30:12.345Z`（UTC，不引 chrono）。
fn now() -> String {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = elapsed.as_secs();
    let (y, m, d) = civil_from_days((secs / 86_400) as i64);
    let rem = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}.{:03}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60,
        elapsed.subsec_millis()
    )
}

/// 1970-01-01 起的天数 → 年月日（Howard Hinnant 的 civil_from_days）。
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }
}
