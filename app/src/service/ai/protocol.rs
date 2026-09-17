//! 一个「接口格式」= 地址怎么补 + 认证头怎么写 + 请求体什么形状 + 响应怎么取文本 + 模型列表。
//! 四家的差别全在这里（从旧 `ai/protocol.js` 移植）。

use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::error::Error;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    #[default]
    OpenaiChat,
    OpenaiResponses,
    Anthropic,
    Google,
}

/// 一条消息。`role` 是 `system` / `user` / `assistant`。
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

/// 旧参数可以调生成参数，但不能替换翻译消息、固定模型、非流式传输和结构化输出约定。
const FIXED_ARGUMENTS: [&str; 24] = [
    "model",
    "messages",
    "input",
    "instructions",
    "system",
    "systemInstruction",
    "contents",
    "generationConfig",
    "stream",
    "stream_options",
    "response_format",
    "text",
    "tools",
    "tool_choice",
    "functions",
    "function_call",
    "parallel_tool_calls",
    "previous_response_id",
    "conversation",
    "prompt",
    "stop",
    "stop_sequences",
    "modalities",
    "audio",
];

const ANTHROPIC_MAX_TOKENS: i64 = 4096;

pub fn compatible_arguments(args: &Map<String, Value>) -> Map<String, Value> {
    args.iter()
        .filter(|(k, _)| !FIXED_ARGUMENTS.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect()
}

fn number(args: &Map<String, Value>, key: &str) -> Option<Value> {
    args.get(key).filter(|v| v.is_number()).cloned()
}

/// 实际发出去的参数（也是缓存身份的一部分）：Anthropic / Google 只认 temperature、top_p、max_tokens，
/// 其余丢掉，免得被忽略的旧字段造成不同的缓存身份。
pub fn arguments_for(protocol: Protocol, args: &Map<String, Value>) -> Map<String, Value> {
    let compatible = compatible_arguments(args);
    if !matches!(protocol, Protocol::Anthropic | Protocol::Google) {
        return compatible;
    }
    let mut out = Map::new();
    for key in ["temperature", "top_p", "max_tokens"] {
        if let Some(v) = number(&compatible, key) {
            out.insert(key.into(), v);
        }
    }
    if protocol == Protocol::Anthropic && !out.contains_key("max_tokens") {
        out.insert("max_tokens".into(), ANTHROPIC_MAX_TOKENS.into());
    }
    out
}

/// 地址末尾可能已经带着的端点段，统一剥掉再自己拼。
const KNOWN_TAILS: [&str; 4] = ["/chat/completions", "/responses", "/messages", "/models"];

/// 用户手填的地址形态很自由：完整端点、只到 `/v1`、只有 base、没写协议头、挂在子路径上。
/// 已经带版本号（`/v1`、`/v2`、`/v1beta`）就不再补。
fn resolve(base: &str, tail: &str, version: &str) -> Result<String, Error> {
    let base = base.trim();
    let full = if base.starts_with("http://") || base.starts_with("https://") {
        base.to_owned()
    } else {
        format!("https://{base}")
    };
    let (scheme, rest) = full
        .split_once("://")
        .ok_or(Error::NotConfigured("base_url"))?;
    let split = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let (authority, after) = rest.split_at(split);
    if authority.is_empty() {
        return Err(Error::NotConfigured("base_url"));
    }
    let suffix_at = after.find(['?', '#']).unwrap_or(after.len());
    let (path, suffix) = after.split_at(suffix_at);
    let mut path = path.strip_suffix('/').unwrap_or(path);
    if let Some(tail) = KNOWN_TAILS.iter().find(|t| path.ends_with(*t)) {
        path = &path[..path.len() - tail.len()];
    }
    let versioned = path.rsplit_once('/').is_some_and(|(_, last)| {
        last.strip_prefix('v').is_some_and(|v| {
            let digits = v.trim_end_matches(|c: char| c.is_ascii_lowercase());
            !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
        })
    });
    let version = if versioned {
        String::new()
    } else {
        format!("/{version}")
    };
    Ok(format!(
        "{scheme}://{authority}{path}{version}/{tail}{suffix}"
    ))
}

fn system_and_rest(messages: &[Message]) -> (String, Vec<&Message>) {
    let system: Vec<&str> = messages
        .iter()
        .filter(|m| m.role == "system")
        .map(|m| m.content.as_str())
        .collect();
    let rest = messages.iter().filter(|m| m.role != "system").collect();
    (system.join("\n\n"), rest)
}

fn joined_texts(items: Option<&Value>, path: &str) -> Option<String> {
    let parts = items?.as_array()?;
    Some(
        parts
            .iter()
            .filter_map(|p| p.pointer(path).and_then(Value::as_str))
            .collect(),
    )
}

impl Protocol {
    /// 配置里的名字。认不出（含旧配置没有这个字段）一律按 OpenAI Chat Completions。
    pub fn from_name(name: &str) -> Protocol {
        match name {
            "openai_responses" => Protocol::OpenaiResponses,
            "anthropic" => Protocol::Anthropic,
            "google" => Protocol::Google,
            _ => Protocol::OpenaiChat,
        }
    }

    pub fn chat_url(self, base: &str, model: &str) -> Result<String, Error> {
        match self {
            Protocol::OpenaiChat => resolve(base, "chat/completions", "v1"),
            Protocol::OpenaiResponses => resolve(base, "responses", "v1"),
            Protocol::Anthropic => resolve(base, "messages", "v1"),
            // 只有这家把模型名塞在地址里
            Protocol::Google => resolve(base, &format!("models/{model}:generateContent"), "v1beta"),
        }
    }

    pub fn models_url(self, base: &str) -> Result<String, Error> {
        match self {
            Protocol::OpenaiChat | Protocol::OpenaiResponses | Protocol::Anthropic => {
                resolve(base, "models", "v1")
            }
            Protocol::Google => resolve(base, "models", "v1beta"),
        }
    }

    pub fn headers(self, api_key: &str) -> Vec<(&'static str, String)> {
        match self {
            Protocol::OpenaiChat | Protocol::OpenaiResponses => {
                vec![("Authorization", format!("Bearer {api_key}"))]
            }
            Protocol::Anthropic => vec![
                ("x-api-key", api_key.to_owned()),
                ("anthropic-version", "2023-06-01".to_owned()),
            ],
            Protocol::Google => vec![("x-goog-api-key", api_key.to_owned())],
        }
    }

    /// `args` 应该已经过 [`arguments_for`]；这里再按各家形状挑一遍。
    pub fn body(self, model: &str, messages: &[Message], args: &Map<String, Value>) -> Value {
        match self {
            Protocol::OpenaiChat | Protocol::OpenaiResponses => {
                let mut body = compatible_arguments(args);
                body.insert("stream".into(), false.into());
                body.insert("model".into(), model.into());
                let key = if self == Protocol::OpenaiChat {
                    "messages"
                } else {
                    // Responses 用 input 而不是 messages，形状照样是 {role, content}
                    "input"
                };
                body.insert(key.into(), json!(messages));
                Value::Object(body)
            }
            Protocol::Anthropic => {
                let (system, rest) = system_and_rest(messages);
                let mut body = Map::new();
                body.insert("model".into(), model.into());
                body.insert("stream".into(), false.into());
                // Anthropic 必填
                let max = number(args, "max_tokens").unwrap_or(ANTHROPIC_MAX_TOKENS.into());
                body.insert("max_tokens".into(), max);
                if !system.is_empty() {
                    body.insert("system".into(), system.into());
                }
                body.insert("messages".into(), json!(rest));
                // frequency_penalty / presence_penalty 传过去直接 400，只挑认识的
                for key in ["temperature", "top_p"] {
                    if let Some(v) = number(args, key) {
                        body.insert(key.into(), v);
                    }
                }
                Value::Object(body)
            }
            Protocol::Google => {
                let (system, rest) = system_and_rest(messages);
                let mut body = Map::new();
                if !system.is_empty() {
                    body.insert(
                        "systemInstruction".into(),
                        json!({ "parts": [{ "text": system }] }),
                    );
                }
                let contents: Vec<Value> = rest
                    .iter()
                    .map(|m| {
                        let role = if m.role == "assistant" {
                            "model"
                        } else {
                            "user"
                        };
                        json!({ "role": role, "parts": [{ "text": m.content }] })
                    })
                    .collect();
                body.insert("contents".into(), contents.into());
                let mut generation = Map::new();
                for (from, to) in [
                    ("temperature", "temperature"),
                    ("top_p", "topP"),
                    ("max_tokens", "maxOutputTokens"),
                ] {
                    if let Some(v) = number(args, from) {
                        generation.insert(to.into(), v);
                    }
                }
                body.insert("generationConfig".into(), generation.into());
                Value::Object(body)
            }
        }
    }

    /// 从响应里取模型输出的文本。取不到返回 `None`，调用方按「空结果」处理，不许崩。
    pub fn text(self, data: &Value) -> Option<String> {
        match self {
            Protocol::OpenaiChat => data
                .pointer("/choices/0/message/content")
                .and_then(Value::as_str)
                .map(str::to_owned),
            Protocol::OpenaiResponses => {
                if let Some(text) = data.get("output_text").and_then(Value::as_str) {
                    return Some(text.to_owned());
                }
                let output = data.get("output")?.as_array()?;
                Some(
                    output
                        .iter()
                        .filter_map(|o| joined_texts(o.get("content"), "/text"))
                        .collect(),
                )
            }
            Protocol::Anthropic => joined_texts(data.get("content"), "/text"),
            Protocol::Google => joined_texts(data.pointer("/candidates/0/content/parts"), "/text"),
        }
    }

    /// 拿不到约定形状返回 None，调用方按「空结果」处理，不许崩（跟 `text()` 一个规矩）。
    pub fn models(self, data: &Value) -> Option<Vec<String>> {
        match self {
            Protocol::OpenaiChat | Protocol::OpenaiResponses | Protocol::Anthropic => {
                let items = data.get("data")?.as_array()?;
                let list = items
                    .iter()
                    .filter_map(|m| m.get("id").and_then(Value::as_str).map(str::to_owned))
                    .collect();
                Some(list)
            }
            Protocol::Google => {
                let items = data.get("models")?.as_array()?;
                let list = items
                    .iter()
                    .filter_map(|m| {
                        let name = m.get("name").and_then(Value::as_str)?;
                        Some(name.strip_prefix("models/").unwrap_or(name).to_owned())
                    })
                    .collect();
                Some(list)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(base: &str) -> String {
        Protocol::OpenaiChat.chat_url(base, "m").unwrap()
    }
    fn models(base: &str) -> String {
        resolve(base, "models", "v1").unwrap()
    }

    #[test]
    fn urls_accept_every_shape_users_type() {
        // 完整的 completions 地址：models 换掉最后一段，chat 原样
        assert_eq!(
            models("https://api.deepseek.com/v1/chat/completions"),
            "https://api.deepseek.com/v1/models"
        );
        assert_eq!(
            chat("https://api.deepseek.com/v1/chat/completions"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        // 只给 base：补 /v1/...
        assert_eq!(
            models("https://api.openai.com"),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            chat("https://api.openai.com"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            models("https://api.openai.com/"),
            "https://api.openai.com/v1/models"
        );
        // 挂在子路径上的端点
        assert_eq!(
            models("https://host/api/v1/chat/completions"),
            "https://host/api/v1/models"
        );
        assert_eq!(models("https://host/api"), "https://host/api/v1/models");
        assert_eq!(
            chat("https://host/api"),
            "https://host/api/v1/chat/completions"
        );
        // 没写协议头
        assert_eq!(models("api.openai.com"), "https://api.openai.com/v1/models");
        assert_eq!(
            chat("api.openai.com"),
            "https://api.openai.com/v1/chat/completions"
        );
        // 只给到 /v1：不能再补一层
        assert_eq!(
            chat("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            models("https://api.deepseek.com/v1"),
            "https://api.deepseek.com/v1/models"
        );
        assert_eq!(
            chat("https://api.deepseek.com/v1/"),
            "https://api.deepseek.com/v1/chat/completions"
        );
        assert_eq!(
            chat("https://host/api/v2"),
            "https://host/api/v2/chat/completions"
        );
        // 本地端口
        assert_eq!(
            chat("http://127.0.0.1:8045/"),
            "http://127.0.0.1:8045/v1/chat/completions"
        );
        assert_eq!(
            chat("http://127.0.0.1:8045"),
            "http://127.0.0.1:8045/v1/chat/completions"
        );
        assert_eq!(
            models("http://127.0.0.1:8045/"),
            "http://127.0.0.1:8045/v1/models"
        );
        // 查询串原样留在后面
        assert_eq!(
            chat(" https://h/v1?api-version=2 "),
            "https://h/v1/chat/completions?api-version=2"
        );
        assert!(matches!(
            Protocol::OpenaiChat.chat_url("  ", "m"),
            Err(Error::NotConfigured("base_url"))
        ));
    }

    #[test]
    fn four_protocol_urls() {
        assert_eq!(
            Protocol::OpenaiResponses
                .chat_url("https://api.openai.com", "m")
                .unwrap(),
            "https://api.openai.com/v1/responses"
        );
        assert_eq!(
            Protocol::Anthropic
                .chat_url("https://api.anthropic.com", "m")
                .unwrap(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            Protocol::Google
                .chat_url(
                    "https://generativelanguage.googleapis.com",
                    "gemini-2.0-flash"
                )
                .unwrap(),
            "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent"
        );
        // v1beta 也算版本号
        assert_eq!(
            resolve("https://host/x/v1beta", "models", "v1beta").unwrap(),
            "https://host/x/v1beta/models"
        );
        // 已经填了别家的端点段，换格式时要能剥掉重拼
        assert_eq!(
            Protocol::Anthropic
                .chat_url("https://host/v1/chat/completions", "m")
                .unwrap(),
            "https://host/v1/messages"
        );
    }

    #[test]
    fn auth_headers() {
        assert_eq!(
            Protocol::OpenaiChat.headers("k"),
            [("Authorization", "Bearer k".to_owned())]
        );
        assert_eq!(
            Protocol::Anthropic.headers("k"),
            [
                ("x-api-key", "k".to_owned()),
                ("anthropic-version", "2023-06-01".to_owned())
            ]
        );
        assert_eq!(
            Protocol::Google.headers("k"),
            [("x-goog-api-key", "k".to_owned())]
        );
    }

    fn prompt() -> Vec<Message> {
        vec![
            Message {
                role: "system",
                content: "sys".into(),
            },
            Message {
                role: "user",
                content: "hi".into(),
            },
        ]
    }
    fn args(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn bodies_per_protocol() {
        let a = args(
            json!({"temperature": 0.1, "top_p": 0.99, "frequency_penalty": 0, "presence_penalty": 0}),
        );
        let p = prompt();
        // OpenAI 两家：system 留在消息列表里，参数原样透传
        let chat = Protocol::OpenaiChat.body("m", &p, &a);
        assert_eq!(chat["messages"], json!(p));
        assert_eq!(chat["frequency_penalty"], json!(0));
        assert_eq!(
            Protocol::OpenaiResponses.body("m", &p, &a)["input"],
            json!(p)
        );
        // Anthropic：system 单独一个字段，max_tokens 必填，penalty 不能传
        let ab = Protocol::Anthropic.body("m", &p, &a);
        assert_eq!(ab["system"], "sys");
        assert_eq!(ab["messages"], json!([{"role": "user", "content": "hi"}]));
        assert_eq!(ab["max_tokens"], 4096);
        assert_eq!(
            Protocol::Anthropic.body("m", &p, &args(json!({"max_tokens": 5})))["max_tokens"],
            5
        );
        assert!(ab.get("frequency_penalty").is_none() && ab.get("presence_penalty").is_none());
        // Google：systemInstruction + contents，参数搬进 generationConfig 并改名
        let gb = Protocol::Google.body("m", &p, &a);
        assert_eq!(gb["systemInstruction"], json!({"parts": [{"text": "sys"}]}));
        assert_eq!(
            gb["contents"],
            json!([{"role": "user", "parts": [{"text": "hi"}]}])
        );
        assert_eq!(gb["generationConfig"]["topP"], 0.99);
        assert!(gb["generationConfig"].get("top_p").is_none());
        assert!(gb["generationConfig"].get("frequency_penalty").is_none());
    }

    #[test]
    fn response_text() {
        assert_eq!(
            Protocol::OpenaiChat
                .text(&json!({"choices": [{"message": {"content": "a"}}]}))
                .as_deref(),
            Some("a")
        );
        assert_eq!(
            Protocol::OpenaiResponses
                .text(&json!({"output": [{"content": [{"text": "a"}, {"text": "b"}]}]}))
                .as_deref(),
            Some("ab")
        );
        assert_eq!(
            Protocol::OpenaiResponses
                .text(&json!({"output_text": "x"}))
                .as_deref(),
            Some("x")
        );
        assert_eq!(
            Protocol::Anthropic
                .text(&json!({"content": [{"text": "a"}]}))
                .as_deref(),
            Some("a")
        );
        assert_eq!(
            Protocol::Google
                .text(&json!({"candidates": [{"content": {"parts": [{"text": "a"}]}}]}))
                .as_deref(),
            Some("a")
        );
        // 取不到就是 None，不许崩
        assert_eq!(Protocol::OpenaiChat.text(&json!({"error": "x"})), None);
        assert_eq!(Protocol::Anthropic.text(&json!("plain")), None);
    }

    #[test]
    fn unknown_names_fall_back_to_openai_chat() {
        assert_eq!(Protocol::from_name(""), Protocol::OpenaiChat);
        assert_eq!(Protocol::from_name("nonsense"), Protocol::OpenaiChat);
        assert_eq!(Protocol::from_name("anthropic"), Protocol::Anthropic);
    }

    #[test]
    fn fixed_fields_win_over_old_arguments() {
        let conflicting = args(json!({
            "model": "wrong", "stream": true, "messages": ["wrong"], "input": "wrong",
            "instructions": "wrong", "response_format": {"type": "text"}, "text": {"format": {}},
            "tools": [{"type": "function"}], "tool_choice": "required", "stop": ["}"], "temperature": 0.3,
        }));
        for protocol in [Protocol::OpenaiChat, Protocol::OpenaiResponses] {
            let body = Protocol::body(protocol, "chosen", &prompt(), &conflicting);
            assert_eq!(body["model"], "chosen");
            assert_eq!(body["stream"], false);
            let (own, other) = if protocol == Protocol::OpenaiChat {
                ("messages", "input")
            } else {
                ("input", "messages")
            };
            assert_eq!(body[own], json!(prompt()));
            assert!(body.get(other).is_none());
            assert_eq!(body["temperature"], 0.3);
            for forbidden in [
                "instructions",
                "response_format",
                "text",
                "tools",
                "tool_choice",
                "stop",
            ] {
                assert!(body.get(forbidden).is_none(), "{forbidden}");
            }
        }
        assert_eq!(
            Protocol::Anthropic.body("m", &prompt(), &args(json!({"stream": true})))["stream"],
            false
        );
    }

    #[test]
    fn arguments_per_protocol() {
        let empty = Map::new();
        assert_eq!(
            Value::Object(arguments_for(Protocol::Anthropic, &empty)),
            json!({"max_tokens": 4096})
        );
        assert_eq!(
            Value::Object(arguments_for(
                Protocol::Google,
                &args(json!({"frequency_penalty": 1}))
            )),
            json!({})
        );
        assert_eq!(
            Value::Object(arguments_for(
                Protocol::OpenaiChat,
                &args(json!({"a": 1, "model": "x"}))
            )),
            json!({"a": 1})
        );
    }

    #[test]
    fn models_url_accepts_various_shapes() {
        // 完整 completions 地址
        assert_eq!(
            Protocol::OpenaiChat
                .models_url("https://api.deepseek.com/v1/chat/completions")
                .unwrap(),
            "https://api.deepseek.com/v1/models"
        );
        // 只到 base
        assert_eq!(
            Protocol::OpenaiChat
                .models_url("https://api.openai.com")
                .unwrap(),
            "https://api.openai.com/v1/models"
        );
        // 带子路径
        assert_eq!(
            Protocol::Anthropic
                .models_url("https://host/api/v1/messages")
                .unwrap(),
            "https://host/api/v1/models"
        );
        // 带查询串
        assert_eq!(
            Protocol::OpenaiResponses
                .models_url("https://h/v1?api-version=2")
                .unwrap(),
            "https://h/v1/models?api-version=2"
        );
        // Google 使用 v1beta
        assert_eq!(
            Protocol::Google
                .models_url("https://generativelanguage.googleapis.com")
                .unwrap(),
            "https://generativelanguage.googleapis.com/v1beta/models"
        );
    }

    #[test]
    fn parses_model_lists() {
        // OpenAI / Anthropic 形状 (data[].id)
        let openai_data = json!({
            "data": [
                { "id": "gpt-4o", "object": "model" },
                { "id": "gpt-3.5-turbo", "object": "model" }
            ]
        });
        assert_eq!(
            Protocol::OpenaiChat.models(&openai_data),
            Some(vec!["gpt-4o".into(), "gpt-3.5-turbo".into()])
        );
        assert_eq!(
            Protocol::Anthropic.models(&openai_data),
            Some(vec!["gpt-4o".into(), "gpt-3.5-turbo".into()])
        );

        // Google 形状 (models[].name，去掉 models/ 前缀)
        let google_data = json!({
            "models": [
                { "name": "models/gemini-2.0-flash", "displayName": "Gemini 2.0 Flash" },
                { "name": "gemini-1.5-pro", "displayName": "Gemini 1.5 Pro" }
            ]
        });
        assert_eq!(
            Protocol::Google.models(&google_data),
            Some(vec!["gemini-2.0-flash".into(), "gemini-1.5-pro".into()])
        );

        // 形状不对返回 None
        assert_eq!(
            Protocol::OpenaiChat.models(&json!({ "error": "unauthorized" })),
            None
        );
        assert_eq!(
            Protocol::OpenaiChat.models(&json!({ "data": "not an array" })),
            None
        );
        assert_eq!(Protocol::Google.models(&json!({ "models": null })), None);
    }
}
