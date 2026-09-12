use crate::APP;
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Mutex,
};
use tauri::{Manager, Runtime};
use tauri_plugin_store::{with_store, Store, StoreCollection};

type Result<T> = std::result::Result<T, tauri_plugin_store::Error>;
const RETIRED: &[&str] = &[
    "translate_auto_copy",
    "translate_delete_newline",
    "translate_code_split",
    "wordbook_ai_instance",
    "recognize_google",
];

#[derive(Clone, Default)]
struct Versions {
    revision: u64,
    generations: HashMap<String, u64>,
    deleted: HashSet<String>,
}

pub struct ConfigState {
    path: PathBuf,
    first_run: bool,
    // Always accessed inside with_store, never in the reverse lock order.
    versions: Mutex<Versions>,
}

#[derive(Clone, Serialize)]
pub struct ConfigSnapshot {
    values: HashMap<String, Value>,
    generations: HashMap<String, u64>,
    deleted: HashSet<String>,
    revision: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigOperation {
    kind: String,
    key: String,
    value: Option<Value>,
    #[serde(default)]
    generation: u64,
    #[serde(default)]
    invalidate: bool,
    #[serde(default)]
    revive: bool,
}

fn invalid(message: &str) -> tauri_plugin_store::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message).into()
}

fn snapshot<R: Runtime>(store: &Store<R>, versions: &Versions) -> ConfigSnapshot {
    ConfigSnapshot {
        values: store
            .entries()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        generations: versions.generations.clone(),
        deleted: versions.deleted.clone(),
        revision: versions.revision,
    }
}

// Only config_committed means persistence succeeded, never store://change.
// Plugin mutations update the cache before emitting; finish every rollback
// operation even if an event recipient is unavailable.
fn transact<R: Runtime, F>(store: &mut Store<R>, save: bool, change: F) -> Result<()>
where
    F: FnOnce(&mut Store<R>) -> Result<()>,
{
    let before: HashMap<_, _> = store
        .entries()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    if let Err(error) = change(store).and_then(|_| if save { store.save() } else { Ok(()) }) {
        let added: Vec<_> = store
            .keys()
            .filter(|k| !before.contains_key(*k))
            .cloned()
            .collect();
        for key in added {
            let _ = store.delete(key);
        }
        for (key, value) in before {
            if store.get(&key) != Some(&value) {
                let _ = store.insert(key, value);
            }
        }
        return Err(error);
    }
    Ok(())
}

fn migrated(mut values: HashMap<String, Value>) -> HashMap<String, Value> {
    for key in RETIRED {
        values.remove(*key);
    }
    // Mirrors src/services/translate/index.jsx and utils/recognize.js.
    for (key, builtin, prefix) in [
        (
            "translate_service_list",
            &["baidu", "bing", "deepl", "google", "transmart", "ai"][..],
            "",
        ),
        (
            "recognize_service_list",
            &["wechat", "umi"][..],
            "recognize_",
        ),
    ] {
        if let Some(Value::Array(list)) = values.get(key).cloned() {
            let mut kept = Vec::new();
            for item in list {
                if let Some(name) = item.as_str() {
                    if builtin.contains(&name.split('@').next().unwrap_or("")) {
                        kept.push(item);
                    } else {
                        values.remove(&format!("{}{}", prefix, name));
                    }
                } else {
                    kept.push(item);
                }
            }
            values.insert(key.into(), Value::Array(kept));
        }
    }
    values
}

fn replace<R: Runtime>(store: &mut Store<R>, values: &HashMap<String, Value>) -> Result<()> {
    let removed: Vec<_> = store
        .keys()
        .filter(|key| !values.contains_key(*key))
        .cloned()
        .collect();
    for key in removed {
        store.delete(key)?;
    }
    for (key, value) in values {
        if store.get(key) != Some(value) {
            store.insert(key.clone(), value.clone())?;
        }
    }
    Ok(())
}

pub fn init_config(app: &mut tauri::App) -> Result<()> {
    // Resolve once on the native side. Every webview receives this exact path.
    let path = app
        .path_resolver()
        .app_config_dir()
        .ok_or_else(|| invalid("Config directory is unavailable"))?
        .join("config.json");
    let handle = app.handle();
    let first_run = with_store(
        handle.clone(),
        handle.state::<StoreCollection<_>>(),
        &path,
        |store| {
            let first_run = store.is_empty();
            let values: HashMap<_, _> = store
                .entries()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            let clean = migrated(values.clone());
            if clean != values || !path.exists() {
                transact(store, true, |store| replace(store, &clean))?;
            }
            Ok(first_run)
        },
    )?;
    app.manage(ConfigState {
        path,
        first_run,
        versions: Mutex::new(Versions {
            deleted: RETIRED.iter().map(|s| s.to_string()).collect(),
            ..Versions::default()
        }),
    });
    info!("Config: shared store initialized");
    Ok(())
}

pub fn get(key: &str) -> Option<Value> {
    let app = APP.get()?;
    let state = app.state::<ConfigState>();
    with_store(
        app.clone(),
        app.state::<StoreCollection<_>>(),
        &state.path,
        |store| Ok(store.get(key).cloned()),
    )
    .ok()
    .flatten()
}

pub fn is_first_run() -> bool {
    APP.get().unwrap().state::<ConfigState>().first_run
}

#[tauri::command]
pub fn config_path(app: tauri::AppHandle) -> PathBuf {
    app.state::<ConfigState>().path.clone()
}

#[tauri::command(async)]
pub fn config_snapshot<R: Runtime>(app: tauri::AppHandle<R>) -> Result<ConfigSnapshot> {
    let state = app.state::<ConfigState>();
    with_store(
        app.clone(),
        app.state::<StoreCollection<_>>(),
        &state.path,
        |store| Ok(snapshot(store, &state.versions.lock().unwrap())),
    )
}

#[tauri::command(async)]
pub fn config_commit<R: Runtime>(
    app: tauri::AppHandle<R>,
    operations: Vec<ConfigOperation>,
) -> Result<ConfigSnapshot> {
    let state = app.state::<ConfigState>();
    with_store(
        app.clone(),
        app.state::<StoreCollection<_>>(),
        &state.path,
        |store| {
            let mut versions = state.versions.lock().unwrap();
            let mut next = versions.clone();
            // Validate the entire intent before changing any cache entry.
            for op in &operations {
                if !["set", "delete", "setIfAbsent"].contains(&op.kind.as_str()) {
                    return Err(invalid("Unknown configuration operation"));
                }
                if op.kind == "setIfAbsent" {
                    continue;
                }
                if op.generation != *versions.generations.get(&op.key).unwrap_or(&0) {
                    return Err(invalid(
                        "Configuration changed. Reopen the setting and try again.",
                    ));
                }
                if op.kind == "set"
                    && (RETIRED.contains(&op.key.as_str())
                        || (versions.deleted.contains(&op.key) && !op.revive))
                {
                    return Err(invalid(
                        "This configuration was removed. Add the service again to save it.",
                    ));
                }
            }
            let before = snapshot(store, &versions).values;
            let mut values = before.clone();
            for op in operations {
                match op.kind.as_str() {
                    "delete" => {
                        values.remove(&op.key);
                        next.deleted.insert(op.key.clone());
                    }
                    "set" => {
                        values.insert(op.key.clone(), op.value.unwrap_or(Value::Null));
                        if op.revive {
                            next.deleted.remove(&op.key);
                        }
                    }
                    "setIfAbsent" => {
                        if !next.deleted.contains(&op.key) && !RETIRED.contains(&op.key.as_str()) {
                            values
                                .entry(op.key.clone())
                                .or_insert(op.value.unwrap_or(Value::Null));
                        }
                    }
                    _ => unreachable!(),
                }
                if op.kind == "delete" || op.invalidate || op.revive {
                    *next.generations.entry(op.key).or_default() += 1;
                }
            }
            if values != before {
                transact(store, true, |store| replace(store, &values))?;
            }
            next.revision += 1;
            *versions = next;
            let result = snapshot(store, &versions);
            if let Err(error) = app.emit_all("config_committed", &result) {
                warn!("Config: committed notification failed: {}", error);
            }
            Ok(result)
        },
    )
}

#[tauri::command(async)]
pub fn reload_store<R: Runtime>(app: tauri::AppHandle<R>) -> Result<ConfigSnapshot> {
    let state = app.state::<ConfigState>();
    with_store(
        app.clone(),
        app.state::<StoreCollection<_>>(),
        &state.path,
        |store| {
            // Read/parse under the save lock. load() would merge deleted disk keys.
            let disk: HashMap<String, Value> =
                serde_json::from_slice(&std::fs::read(&state.path)?)?;
            let clean = migrated(disk.clone());
            let mut versions = state.versions.lock().unwrap();
            let before = snapshot(store, &versions).values;
            if before == clean && clean == disk {
                return Ok(snapshot(store, &versions));
            }
            transact(store, clean != disk, |store| replace(store, &clean))?;
            for key in before.keys().chain(clean.keys()).collect::<HashSet<_>>() {
                if before.get(key) != clean.get(key) {
                    *versions.generations.entry(key.clone()).or_default() += 1;
                    if clean.contains_key(key) {
                        versions.deleted.remove(key);
                    } else {
                        versions.deleted.insert(key.clone());
                    }
                }
            }
            versions.revision += 1;
            let result = snapshot(store, &versions);
            if let Err(error) = app.emit_all("config_committed", &result) {
                warn!("Config: reload notification failed: {}", error);
            }
            Ok(result)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use tauri::test::{mock_builder, mock_context, noop_assets, MockRuntime};

    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);

    fn fixture() -> (tauri::App<MockRuntime>, PathBuf) {
        let directory = std::env::temp_dir().join(format!(
            "tilex-config-test-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("config.json");
        std::fs::write(&path, b"{}").unwrap();
        let app = mock_builder()
            .plugin(tauri_plugin_store::Builder::default().build())
            .build(mock_context(noop_assets()))
            .unwrap();
        app.manage(ConfigState {
            path: path.clone(),
            first_run: false,
            versions: Mutex::new(Versions::default()),
        });
        (app, path)
    }

    fn ops(value: Value) -> Vec<ConfigOperation> {
        serde_json::from_value(value).unwrap()
    }
    fn disk(path: &PathBuf) -> Value {
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }
    fn cleanup(path: PathBuf) {
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_dir(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn shared_plugin_store_serializes_commits_and_defaults() {
        let (app, path) = fixture();
        let a = app.handle();
        let b = app.handle();
        let first = std::thread::spawn(move || {
            config_commit(
                a,
                ops(json!([
                    {"kind":"set", "key":"one", "value":"new"},
                    {"kind":"set", "key":"two", "value":2}
                ])),
            )
            .unwrap()
        });
        let second = std::thread::spawn(move || {
            config_commit(
                b,
                ops(json!([
                    {"kind":"set", "key":"three", "value":3}
                ])),
            )
            .unwrap()
        });
        first.join().unwrap();
        second.join().unwrap();
        config_commit(
            app.handle(),
            ops(json!([{"kind":"setIfAbsent", "key":"one", "value":"old"}])),
        )
        .unwrap();
        assert_eq!(disk(&path), json!({"one":"new", "two":2, "three":3}));
        // This is the installed plugin's own public accessor, not a duplicate map.
        let one = with_store(
            app.handle(),
            app.state::<StoreCollection<_>>(),
            &path,
            |store| Ok(store.get("one").cloned()),
        )
        .unwrap();
        assert_eq!(one, Some(json!("new")));
        cleanup(path);
    }

    #[test]
    fn deleted_generation_rejects_cross_window_stale_list_and_value() {
        let (app, path) = fixture();
        config_commit(
            app.handle(),
            ops(json!([
                {"kind":"set", "key":"list", "value":["a","b"]},
                {"kind":"set", "key":"a", "value":{"token":"test-only"}}
            ])),
        )
        .unwrap();
        config_commit(
            app.handle(),
            ops(json!([
                {"kind":"set", "key":"list", "value":["b"], "invalidate":true},
                {"kind":"delete", "key":"a"}
            ])),
        )
        .unwrap();
        assert!(config_commit(
            app.handle(),
            ops(json!([
                {"kind":"set", "key":"list", "value":["a","b"]}
            ]))
        )
        .is_err());
        assert!(config_commit(
            app.handle(),
            ops(json!([
                {"kind":"set", "key":"a", "value":{"old":true}}
            ]))
        )
        .is_err());
        config_commit(
            app.handle(),
            ops(json!([{"kind":"setIfAbsent", "key":"a", "value":{}}])),
        )
        .unwrap();
        assert_eq!(disk(&path), json!({"list":["b"]}));
        config_commit(
            app.handle(),
            ops(json!([
                {"kind":"set", "key":"a", "value":{"new":true}, "generation":1, "revive":true},
                {"kind":"set", "key":"list", "value":["b","a"], "generation":1, "invalidate":true}
            ])),
        )
        .unwrap();
        assert_eq!(disk(&path)["a"], json!({"new":true}));
        cleanup(path);
    }

    #[test]
    fn save_failure_rolls_back_cache_and_does_not_publish_success() {
        let (app, path) = fixture();
        config_commit(
            app.handle(),
            ops(json!([{"kind":"set", "key":"keep", "value":1}])),
        )
        .unwrap();
        let events = Arc::new(AtomicUsize::new(0));
        let count = events.clone();
        let unlisten = app.listen_global("config_committed", move |_| {
            count.fetch_add(1, Ordering::Relaxed);
        });
        // A directory at the save path deterministically rejects File::create.
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        let failed = config_commit(
            app.handle(),
            ops(json!([
                {"kind":"delete", "key":"keep"}, {"kind":"set", "key":"new", "value":2}
            ])),
        );
        assert!(failed.is_err());
        assert!(!failed.err().unwrap().to_string().trim().is_empty());
        let current = config_snapshot(app.handle()).unwrap();
        assert_eq!(current.values.get("keep"), Some(&json!(1)));
        assert!(!current.values.contains_key("new"));
        assert!(!current.deleted.contains("keep"));
        assert_eq!(events.load(Ordering::Relaxed), 0);
        app.unlisten(unlisten);
        std::fs::remove_dir(&path).unwrap();
        config_commit(
            app.handle(),
            ops(json!([{"kind":"set", "key":"retry", "value":true}])),
        )
        .unwrap();
        cleanup(path);
    }

    #[test]
    fn reload_replaces_snapshot_and_preserves_cache_on_invalid_json() {
        let (app, path) = fixture();
        config_commit(
            app.handle(),
            ops(json!([
                {"kind":"set", "key":"removed", "value":1}, {"kind":"set", "key":"keep", "value":2}
            ])),
        )
        .unwrap();
        std::fs::write(&path, br#"{"keep":3}"#).unwrap();
        let loaded = reload_store(app.handle()).unwrap();
        assert!(!loaded.values.contains_key("removed"));
        assert_eq!(loaded.values.get("keep"), Some(&json!(3)));
        std::fs::write(&path, b"{").unwrap();
        assert!(reload_store(app.handle()).is_err());
        assert_eq!(config_snapshot(app.handle()).unwrap().values, loaded.values);
        cleanup(path);
    }

    #[test]
    fn migration_removes_only_retired_and_unavailable_service_configuration() {
        let before = serde_json::from_value(json!({
            "translate_auto_copy":"source", "translate_delete_newline":true, "translate_code_split":true,
            "wordbook_ai_instance":"old", "recognize_google":{}, "recognize_service_list":["google","wechat"],
            "translate_service_list":["google","gone@1","ai@2"], "gone@1":{"old":true},
            "ai@2":{"apiKey":"test-only"}, "custom_unknown":{"keep":true}
        })).unwrap();
        let after = migrated(before);
        for key in RETIRED {
            assert!(!after.contains_key(*key));
        }
        assert!(!after.contains_key("gone@1"));
        assert_eq!(after["translate_service_list"], json!(["google", "ai@2"]));
        assert_eq!(after["recognize_service_list"], json!(["wechat"]));
        assert_eq!(after["ai@2"], json!({"apiKey":"test-only"}));
        assert_eq!(after["custom_unknown"], json!({"keep":true}));
    }
}
