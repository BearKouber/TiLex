//! R-7：用户目录含中文（本机就是 `C:\Users\威泰普`）时路径不能被替换成 �。
//! 在临时目录下建 `测试_威泰普/` 走一遍。临时 PNG、sidecar 参数随 B3 加进来。

use std::fs;

use tilex::logger;
use tilex::logic::config::Store;
use tilex::logic::saved_entry::Snapshot;
use tilex::logic::wordbook::{self, Db};

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

    // 生词本：在中文目录里建库、写一条中文、读回来。
    let mut db = Db::new(dir.join(wordbook::FILE));
    let entry = Snapshot {
        text: "威泰普".into(),
        translation: "a name".into(),
        detail: None,
        service: "google".into(),
    };
    let id = db.save(1, &entry).unwrap();
    let list = db.list().unwrap();
    assert_eq!((list[0].id, list[0].text.as_str()), (id, "威泰普"));
    assert!(dir.join(wordbook::FILE).exists());
    drop(db);

    fs::remove_dir_all(&base).unwrap();
}
