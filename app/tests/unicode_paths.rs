//! R-7：用户目录含中文（本机就是 `C:\Users\威泰普`）时路径不能被替换成 �。
//! 在临时目录下建 `测试_威泰普/` 走一遍。生词本、临时 PNG、sidecar 参数随 B1/B3/B4 加进来。

use std::fs;

use tilex::logger;
use tilex::logic::config::Store;

#[test]
fn config_and_log_under_chinese_dir() {
    let base = std::env::temp_dir().join(format!("tilex-unicode-{}", std::process::id()));
    let dir = base.join("测试_威泰普");
    let _ = fs::remove_dir_all(&base);

    // 配置：新建 → 保存 → 重新读出来。
    let (store, backup) = Store::open(&dir).unwrap();
    assert!(backup.is_none());
    store.update(|c| c.general.language = "en".into()).unwrap();
    let (reopened, _) = Store::open(&dir).unwrap();
    assert_eq!(reopened.snapshot().general.language, "en");

    // 损坏文件的备份也在中文目录里。
    fs::write(dir.join("config.json"), b"not json").unwrap();
    let (_, backup) = Store::open(&dir).unwrap();
    let backup = backup.unwrap();
    assert!(backup.starts_with(&dir));
    assert_eq!(fs::read(&backup).unwrap(), b"not json");

    // 日志：初始化、写一条、读回来。
    logger::init(&dir).unwrap();
    assert_eq!(logger::dir().unwrap(), dir.join("logs"));
    log::info!("UnicodeTest: 中文日志 ✓");
    let text = fs::read_to_string(dir.join("logs").join("tilex.log")).unwrap();
    assert!(text.contains("UnicodeTest: 中文日志 ✓"), "{text}");

    fs::remove_dir_all(&base).unwrap();
}
