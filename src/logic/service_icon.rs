//! 服务项的「认脸」数据（从旧版 `services/translate/ai/icons.js` 移植）：
//! 厂商图标 id、品牌色、首字，以及从地址 / 模型名推断厂商的规则。
//! 放在 `logic` 而不是 `service::ai`：只吃两个字符串、只给界面用，
//! 而 `src/ui/` 不许依赖 `service`（design §1.2，`tests/source_rules.rs` 在查）。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub color: &'static str,
    pub letter: &'static str,
    pub has_file: bool,
}

pub const FALLBACK_ICON: &str = "sparkle";

pub const ICONS: &[IconInfo] = &[
    IconInfo {
        id: "openai",
        label: "OpenAI",
        color: "#10A37F",
        letter: "O",
        has_file: true,
    },
    IconInfo {
        id: "claude",
        label: "Claude",
        color: "#D97757",
        letter: "C",
        has_file: true,
    },
    IconInfo {
        id: "gemini",
        label: "Gemini",
        color: "#4285F4",
        letter: "G",
        has_file: true,
    },
    IconInfo {
        id: "qwen",
        label: "通义千问",
        color: "#FF6A00",
        letter: "通",
        has_file: true,
    },
    IconInfo {
        id: "copilot",
        label: "Copilot",
        color: "#6E56CF",
        letter: "C",
        has_file: true,
    },
    IconInfo {
        id: "deepseek",
        label: "DeepSeek",
        color: "#4D6BFE",
        letter: "D",
        has_file: true,
    },
    IconInfo {
        id: "ollama",
        label: "Ollama",
        color: "#334155",
        letter: "O",
        has_file: true,
    },
    IconInfo {
        id: "kimi",
        label: "Kimi",
        color: "#16191E",
        letter: "K",
        has_file: true,
    },
    IconInfo {
        id: "zhipu",
        label: "智谱 GLM",
        color: "#3859FF",
        letter: "智",
        has_file: false,
    },
    IconInfo {
        id: "siliconflow",
        label: "SiliconFlow",
        color: "#6E56CF",
        letter: "S",
        has_file: false,
    },
    // MiMo 借小米的厂商 logo，Grok 借同一家的 X logo：simple-icons 都没有模型专属图标
    IconInfo {
        id: "mimo",
        label: "小米 MiMo",
        color: "#FF6900",
        letter: "M",
        has_file: true,
    },
    IconInfo {
        id: "grok",
        label: "Grok",
        color: "#1F2937",
        letter: "X",
        has_file: true,
    },
    IconInfo {
        id: "sparkle",
        label: "AI",
        color: "#8B5CF6",
        letter: "✦",
        has_file: false,
    },
    // 本版内置服务
    IconInfo {
        id: "google",
        label: "Google",
        color: "#4285F4",
        letter: "G",
        has_file: true,
    },
    IconInfo {
        id: "bing",
        label: "Bing",
        color: "#008373",
        letter: "B",
        has_file: true,
    },
    IconInfo {
        id: "deepl",
        label: "DeepL",
        color: "#0F2B46",
        letter: "D",
        has_file: true,
    },
    IconInfo {
        id: "baidu",
        label: "Baidu",
        color: "#2932E1",
        letter: "百",
        has_file: true,
    },
    IconInfo {
        id: "transmart",
        label: "Transmart",
        color: "#0052D9",
        letter: "T",
        has_file: true,
    },
    // 单色图标，用 `color` 上色（旧版也是 react-icons 的 RiWechatFill 配 #07C160）
    IconInfo {
        id: "wechat",
        label: "WeChat",
        color: "#07C160",
        letter: "微",
        has_file: true,
    },
    // Umi-OCR 没有品牌 logo，旧版用的也是通用「文字识别」图标配中性灰（text-default-600）
    IconInfo {
        id: "umi",
        label: "Umi-OCR",
        color: "#52525B",
        letter: "U",
        has_file: true,
    },
];

pub fn get_icon(id: &str) -> &'static IconInfo {
    ICONS
        .iter()
        .find(|i| i.id == id)
        .unwrap_or_else(|| get_fallback_icon())
}

fn get_fallback_icon() -> &'static IconInfo {
    ICONS
        .iter()
        .find(|i| i.id == FALLBACK_ICON)
        .unwrap_or(&ICONS[0])
}

// 关键词 -> 图标 id。顺序必须保证：siliconflow 必须排在 deepseek 前面，
// 否则硅基流动（默认模型是 deepseek-ai/...）会永远顶着 DeepSeek 的脸。
const RULES: &[(&[&str], &str)] = &[
    (&["siliconflow", "siliconcloud"], "siliconflow"),
    (&["deepseek"], "deepseek"),
    (&["anthropic", "claude"], "claude"),
    (&["gemini", "generativelanguage"], "gemini"),
    (&["ollama", "11434"], "ollama"),
    (&["moonshot", "kimi"], "kimi"),
    (&["bigmodel", "zhipu", "glm"], "zhipu"),
    (&["dashscope", "qwen", "aliyun", "tongyi"], "qwen"),
    (&["xiaomimimo", "mimo"], "mimo"),
    (&["grok", "x.ai"], "grok"),
    (&["copilot"], "copilot"),
    (&["openai", "chatgpt", "gpt-"], "openai"),
];

/// 从请求地址和模型名猜厂商。地址比模型名可信，先只拿地址跑一遍，没中再连模型名跑。
pub fn match_icon(request_path: &str, model: &str) -> &'static str {
    let path = request_path.to_lowercase();
    let both = format!("{} {}", path, model.to_lowercase());
    for source in [&path, &both] {
        if source.trim().is_empty() {
            continue;
        }
        for &(patterns, id) in RULES {
            if patterns.iter().any(|&p| source.contains(p)) {
                return id;
            }
        }
    }
    FALLBACK_ICON
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_icon() {
        // 硅基流动挂的是 DeepSeek 的模型，认脸必须按地址走
        assert_eq!(
            match_icon(
                "https://api.siliconflow.cn/v1",
                "deepseek-ai/DeepSeek-V3.2-Exp"
            ),
            "siliconflow"
        );
        // 地址不认识时才轮到模型名
        assert_eq!(
            match_icon("https://my-relay.example.com/v1", "claude-sonnet-5"),
            "claude"
        );
        assert_eq!(match_icon("http://127.0.0.1:11434/v1", ""), "ollama");
        assert_eq!(
            match_icon("https://open.bigmodel.cn/api/paas/v4", "glm-4.6"),
            "zhipu"
        );
        assert_eq!(match_icon("https://api.xiaomimimo.com/v1", ""), "mimo");
        // 都不认识：回落到 sparkle
        assert_eq!(match_icon("", ""), FALLBACK_ICON);
        assert_eq!(
            match_icon("https://unknown.example.com", "my-model"),
            FALLBACK_ICON
        );
    }

    #[test]
    fn test_icons_table() {
        let g = get_icon("google");
        assert_eq!(g.color, "#4285F4");
        assert_eq!(g.letter, "G");
        assert!(g.has_file);

        let w = get_icon("wechat");
        assert_eq!(w.color, "#07C160");
        assert_eq!(w.letter, "微");
        assert!(w.has_file);

        let u = get_icon("umi");
        assert_eq!(u.letter, "U");
        assert!(u.has_file);

        let s = get_icon("siliconflow");
        assert_eq!(s.color, "#6E56CF");
        assert_eq!(s.letter, "S");
        assert!(!s.has_file);

        let d = get_icon("deepseek");
        assert_eq!(d.color, "#4D6BFE");
        assert!(d.has_file);

        let o = get_icon("openai");
        assert_eq!(o.color, "#10A37F");
        assert!(o.has_file);

        let fb = get_icon("nonexistent");
        assert_eq!(fb.id, FALLBACK_ICON);
    }
}
