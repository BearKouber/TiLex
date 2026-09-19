//! 知名 AI 厂商的开箱预设与端点补全（从旧版 `services/translate/ai/presets.js` 移植）。
//! 选完只剩粘 API Key —— 地址和接口格式都是填好的。
//!
//! **刻意不带模型名。** 硬编码的模型一定会过期，而且指定死了反而挡住新模型。
//!
//! 放在 `logic`：包装 `service::ai::protocol`，向 `src/ui/` 提供展示与端点解析接口，
//! 满足 `src/ui/` 不许依赖 `service` 的分层规则。

use crate::service::ai::protocol::Protocol;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub base_url: &'static str,
    pub protocol: &'static str,
    pub help_url: &'static str,
}

pub const AI_PRESETS: &[Preset] = &[
    Preset {
        id: "deepseek",
        name: "DeepSeek",
        base_url: "https://api.deepseek.com",
        protocol: "openai_chat",
        help_url: "https://platform.deepseek.com",
    },
    Preset {
        id: "claude",
        name: "Anthropic Claude",
        base_url: "https://api.anthropic.com",
        protocol: "anthropic",
        help_url: "https://console.anthropic.com",
    },
    Preset {
        id: "gemini",
        name: "Google Gemini",
        base_url: "https://generativelanguage.googleapis.com",
        protocol: "google",
        help_url: "https://aistudio.google.com",
    },
    Preset {
        id: "openai",
        name: "OpenAI",
        base_url: "https://api.openai.com/v1",
        protocol: "openai_chat",
        help_url: "https://platform.openai.com",
    },
    Preset {
        id: "kimi",
        name: "Moonshot Kimi",
        base_url: "https://api.moonshot.cn/v1",
        protocol: "openai_chat",
        help_url: "https://platform.moonshot.cn",
    },
    Preset {
        id: "zhipu",
        name: "智谱 GLM",
        base_url: "https://open.bigmodel.cn/api/paas/v4",
        protocol: "openai_chat",
        help_url: "https://open.bigmodel.cn",
    },
    Preset {
        id: "qwen",
        name: "通义千问",
        base_url: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        protocol: "openai_chat",
        help_url: "https://bailian.console.aliyun.com",
    },
    Preset {
        id: "siliconflow",
        name: "SiliconFlow",
        base_url: "https://api.siliconflow.cn/v1",
        protocol: "openai_chat",
        help_url: "https://cloud.siliconflow.cn",
    },
    Preset {
        id: "mimo",
        name: "小米 MiMo",
        base_url: "https://api.xiaomimimo.com/v1",
        protocol: "openai_chat",
        help_url: "https://mimo.mi.com",
    },
];

pub fn preset_of(id: &str) -> Option<&'static Preset> {
    AI_PRESETS.iter().find(|p| p.id == id)
}

/// 解析出最终发送请求的实际端点 URL。
/// 当 `base` 为空白或解析出错时返回空串。
pub fn resolved_chat_url(base: &str, model: &str, protocol: &str) -> String {
    if base.trim().is_empty() {
        return String::new();
    }
    Protocol::from_name(protocol)
        .chat_url(base.trim(), model.trim())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::service_icon;

    const M: &str = "probe-model";
    const EXPECTED: &[(&str, &str)] = &[
        ("deepseek", "https://api.deepseek.com/v1/chat/completions"),
        ("claude", "https://api.anthropic.com/v1/messages"),
        (
            "gemini",
            "https://generativelanguage.googleapis.com/v1beta/models/probe-model:generateContent",
        ),
        ("openai", "https://api.openai.com/v1/chat/completions"),
        ("kimi", "https://api.moonshot.cn/v1/chat/completions"),
        (
            "zhipu",
            "https://open.bigmodel.cn/api/paas/v4/chat/completions",
        ),
        (
            "qwen",
            "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions",
        ),
        (
            "siliconflow",
            "https://api.siliconflow.cn/v1/chat/completions",
        ),
        ("mimo", "https://api.xiaomimimo.com/v1/chat/completions"),
    ];

    #[test]
    fn test_ai_presets_resolved_chat_url() {
        assert_eq!(AI_PRESETS.len(), EXPECTED.len());
        for p in AI_PRESETS {
            let (_, expected_url) = EXPECTED.iter().find(|(id, _)| *id == p.id).unwrap();
            let resolved = resolved_chat_url(p.base_url, M, p.protocol);
            assert_eq!(&resolved, expected_url, "url mismatch for {}", p.id);

            let icon_info = service_icon::get_icon(p.id);
            assert_eq!(icon_info.id, p.id, "icon not found in ICONS for {}", p.id);
        }
    }

    #[test]
    fn test_preset_of() {
        assert!(preset_of("deepseek").is_some());
        assert_eq!(preset_of("deepseek").unwrap().name, "DeepSeek");
        assert!(preset_of("nonexistent").is_none());
    }

    #[test]
    fn test_resolved_chat_url_empty_base() {
        assert_eq!(resolved_chat_url("", "model", "openai_chat"), "");
        assert_eq!(resolved_chat_url("   ", "model", "openai_chat"), "");
    }
}
