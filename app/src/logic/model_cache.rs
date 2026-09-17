//! 按端点缓存的模型名列表，落盘到 `<数据目录>/models.json`。
//! 缓存不是配置，不进 config.json。写失败只 log::warn，内存里那份照样能用。

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, PoisonError, RwLock};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::logic::config::AiConfig;
use crate::service::ai::protocol::Protocol;

const FILE: &str = "models.json";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub models: Vec<String>,
}

static CACHE: OnceLock<RwLock<BTreeMap<String, Entry>>> = OnceLock::new();

/// 缓存身份 = 模型列表地址。Google 把模型名塞在 chat 地址里，用 chat 地址当 key
/// 会每个模型开一份缓存（旧版 `latency.js:55-57`）。地址填不全时返回空串 = 不缓存。
pub fn endpoint_of(base: &str, protocol: &str) -> String {
    let base = base.trim();
    if base.is_empty() {
        return String::new();
    }
    Protocol::from_name(protocol)
        .models_url(base)
        .unwrap_or_default()
}

fn cache_path() -> Option<PathBuf> {
    crate::platform::data_dir().ok().map(|d| d.join(FILE))
}

fn load_from_slice(bytes: &[u8]) -> BTreeMap<String, Entry> {
    serde_json::from_slice(bytes).unwrap_or_default()
}

fn cache() -> &'static RwLock<BTreeMap<String, Entry>> {
    CACHE.get_or_init(|| {
        let entries = cache_path()
            .and_then(|p| fs::read(p).ok())
            .map(|bytes| load_from_slice(&bytes))
            .unwrap_or_default();
        RwLock::new(entries)
    })
}

/// 之前拉到过的列表，没有就空 Vec。
pub fn models(endpoint: &str) -> Vec<String> {
    if endpoint.is_empty() {
        return Vec::new();
    }
    let guard = cache().read().unwrap_or_else(PoisonError::into_inner);
    guard
        .get(endpoint)
        .map(|e| e.models.clone())
        .unwrap_or_default()
}

/// 把拉到的模型列表更新进内存表。保留该端点未来可能有的其他字段（如测速延迟）。
fn merge_models(table: &mut BTreeMap<String, Entry>, endpoint: &str, models: Vec<String>) {
    table
        .entry(endpoint.to_owned())
        .and_modify(|e| e.models = models.clone())
        .or_insert_with(|| Entry { models });
}

fn write_atomic(path: &Path, table: &BTreeMap<String, Entry>) -> Result<(), Error> {
    if let Some(parent) = path.parent() {
        // ignore: parent directory already exists or create error will surface on file create
        let _ = fs::create_dir_all(parent);
    }
    let bytes = serde_json::to_vec_pretty(table)?;
    let tmp = path.with_extension("json.tmp");
    let result = File::create(&tmp)
        .and_then(|mut f| f.write_all(&bytes).and_then(|()| f.sync_all()))
        .and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp); // ignore: 临时文件删不掉只是留个残渣，下次写会覆盖它
    }
    Ok(result?)
}

fn persist(table: &BTreeMap<String, Entry>) {
    let Some(path) = cache_path() else { return };
    if let Err(e) = write_atomic(&path, table) {
        log::warn!("model_cache: write failed: {e}");
    }
}

/// 拉一次并写进缓存，返回拉到的列表。界面在后台线程上调这个。
pub fn fetch(config: &AiConfig) -> Result<Vec<String>, Error> {
    let list = crate::service::ai::list_models(config)?;
    let endpoint = endpoint_of(&config.base_url, &config.protocol);
    if !endpoint.is_empty() {
        let mut guard = cache().write().unwrap_or_else(PoisonError::into_inner);
        merge_models(&mut guard, &endpoint, list.clone());
        persist(&guard);
    }
    Ok(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_of_rules() {
        // 空地址 -> 空串
        assert_eq!(endpoint_of("", "openai_chat"), "");
        assert_eq!(endpoint_of("   ", "openai_chat"), "");

        // Google 和 OpenAI 两家 -> 不同 key
        let base = "https://api.example.com";
        let openai = endpoint_of(base, "openai_chat");
        let google = endpoint_of(base, "google");
        assert_eq!(openai, "https://api.example.com/v1/models");
        assert_eq!(google, "https://api.example.com/v1beta/models");
        assert_ne!(openai, google);
    }

    #[test]
    fn merge_models_and_retrieve() {
        let mut table = BTreeMap::new();
        let endpoint = "https://api.deepseek.com/v1/models";
        let models_v1 = vec!["deepseek-chat".into(), "deepseek-coder".into()];
        merge_models(&mut table, endpoint, models_v1.clone());
        assert_eq!(table.get(endpoint).unwrap().models, models_v1);

        // 更新覆盖 models
        let models_v2 = vec!["deepseek-chat".into(), "deepseek-reasoner".into()];
        merge_models(&mut table, endpoint, models_v2.clone());
        assert_eq!(table.get(endpoint).unwrap().models, models_v2);
    }

    #[test]
    fn unreadable_file_gives_empty_table() {
        assert_eq!(load_from_slice(b"not json at all"), BTreeMap::new());
        assert_eq!(load_from_slice(b"{\"broken\": "), BTreeMap::new());
        assert_eq!(load_from_slice(b""), BTreeMap::new());
    }
}
