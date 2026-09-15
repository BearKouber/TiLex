//! 翻译结果缓存（design §2.4，从旧 `translate_cache.js` 移植）：内存 LRU 200 条，同一句再划一次不发请求。
//! 键里有请求配置快照（**含 API key**）：只在内存，不写日志、不落盘。

use std::collections::VecDeque;

use serde_json::{Value, json};

pub const CAPACITY: usize = 200;

/// 缓存身份 = 原文 + 源/目标语言 + 服务实例 + 请求开始时冻结的配置快照 + 检测语种。
/// 快照是 `serde_json::Value`：serde_json 没开 `preserve_order`，对象键天然按字母排，
/// 所以字段顺序不同的同一份配置得到同一个键；数组顺序照样算数。
pub fn key(
    text: &str,
    from: &str,
    to: &str,
    service: &str,
    config: &Value,
    detected: &str,
) -> String {
    json!([text, from, to, service, config, detected]).to_string()
}

/// ponytail: 线性查找，200 条、每次划词几次查找，够用；真成瓶颈再换 HashMap + 链表。
pub struct Lru {
    entries: VecDeque<(String, Value)>,
}

impl Lru {
    pub const fn new() -> Self {
        Self {
            entries: VecDeque::new(),
        }
    }

    /// 命中后挪到最新。
    pub fn get(&mut self, key: &str) -> Option<Value> {
        let at = self.entries.iter().position(|(k, _)| k == key)?;
        let entry = self.entries.remove(at)?;
        let value = entry.1.clone();
        self.entries.push_back(entry);
        Some(value)
    }

    pub fn put(&mut self, key: String, value: Value) {
        if let Some(at) = self.entries.iter().position(|(k, _)| *k == key) {
            self.entries.remove(at);
        }
        self.entries.push_back((key, value));
        if self.entries.len() > CAPACITY {
            self.entries.pop_front();
        }
    }
}

impl Default for Lru {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::ai;

    fn ai_key(c: &ai::Config) -> String {
        let snapshot = serde_json::to_value(c.effective()).unwrap();
        key(
            "A sentence to translate.",
            "auto",
            "zh_cn",
            "ai@1",
            &snapshot,
            "en",
        )
    }

    fn fixture() -> ai::Config {
        ai::Config {
            base_url: "https://example.test/v1".into(),
            api_key: "fixture-key".into(),
            model: "fixture-model".into(),
            custom_instructions: "Give an example.".into(),
            request_arguments: json!({"temperature": 0.2, "metadata": {"a": 1, "b": 2}})
                .as_object()
                .unwrap()
                .clone(),
            ..ai::Config::default()
        }
    }

    #[test]
    fn object_key_order_does_not_matter_but_array_order_does() {
        let a = json!({"nested": {"a": 1, "b": 2}});
        let mut b = serde_json::Map::new();
        b.insert("nested".into(), json!({"b": 2, "a": 1}));
        assert_eq!(
            key("t", "a", "b", "s", &a, ""),
            key("t", "a", "b", "s", &b.into(), "")
        );
        assert_ne!(
            key("t", "a", "b", "s", &json!({"l": ["a", "b"]}), ""),
            key("t", "a", "b", "s", &json!({"l": ["b", "a"]}), "")
        );
        // 旧 text_preprocess.test.js：不同服务不能撞键
        assert_ne!(
            key("hi", "en", "zh_cn", "google", &json!({}), ""),
            key("hi", "en", "zh_cn", "bing", &json!({}), "")
        );
        assert_ne!(
            key("hi", "en", "zh_cn", "s", &json!({}), "en"),
            key("hi", "en", "zh_cn", "s", &json!({}), "ja")
        );
    }

    #[test]
    fn every_request_field_changes_the_identity() {
        let base = ai_key(&fixture());
        let changes: [fn(&mut ai::Config); 7] = [
            |c| c.custom_instructions = "Changed instructions".into(),
            |c| c.model = "new-model".into(),
            |c| c.protocol = "openai_responses".into(),
            |c| c.base_url = "https://new.test/v1".into(),
            |c| c.api_key = "rotated-fixture-key".into(),
            |c| c.request_arguments = json!({"temperature": 0.9}).as_object().unwrap().clone(),
            |c| c.legacy_reference_instructions = "Changed migrated references".into(),
        ];
        for change in changes {
            let mut c = fixture();
            change(&mut c);
            assert_ne!(ai_key(&c), base);
        }
        // 显式空要求与缺省要求不同
        let mut empty = fixture();
        empty.custom_instructions.clear();
        let mut default = fixture();
        default.custom_instructions = ai::DEFAULT_CUSTOM_INSTRUCTIONS.into();
        assert_ne!(ai_key(&empty), ai_key(&default));
    }

    #[test]
    fn ignored_arguments_do_not_change_the_identity() {
        let base = ai_key(&fixture());
        let mut noisy = fixture();
        for (k, v) in [
            ("model", json!("ignored")),
            ("messages", json!([])),
            ("stream", json!(true)),
            ("response_format", json!({})),
        ] {
            noisy.request_arguments.insert(k.into(), v);
        }
        assert_eq!(ai_key(&noisy), base);
        let mut google = fixture();
        google.protocol = "google".into();
        google.request_arguments = json!({"temperature": 0.2, "unsupported": true})
            .as_object()
            .unwrap()
            .clone();
        let mut google2 = google.clone();
        google2
            .request_arguments
            .insert("unsupported".into(), false.into());
        assert_eq!(ai_key(&google), ai_key(&google2));
        google2
            .request_arguments
            .insert("temperature".into(), 0.8.into());
        assert_ne!(ai_key(&google), ai_key(&google2));
    }

    #[test]
    fn late_result_keeps_its_own_identity() {
        let mut cache = Lru::new();
        let mut config = fixture();
        let captured = ai_key(&config);
        config.model = "edited-after-start".into();
        cache.put(captured.clone(), json!("late AI result with old settings"));
        assert_eq!(cache.get(&ai_key(&config)), None);
        assert_eq!(
            cache.get(&captured),
            Some(json!("late AI result with old settings"))
        );
    }

    #[test]
    fn lru_evicts_the_oldest() {
        let mut cache = Lru::new();
        for i in 0..=CAPACITY {
            cache.put(format!("lru-{i}"), json!(i));
        }
        assert_eq!(cache.get("lru-0"), None);
        assert_eq!(cache.get("lru-1"), Some(json!(1)));
        cache.put("lru-201".into(), json!(201));
        assert_eq!(cache.get("lru-2"), None, "lru-1 刚被读过，挤掉的是 lru-2");
        assert_eq!(cache.get("lru-1"), Some(json!(1)));
        cache.put("lru-1".into(), json!("new"));
        assert_eq!(cache.get("lru-1"), Some(json!("new")));
    }
}
