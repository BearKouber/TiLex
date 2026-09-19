# 来源与修改声明

## 代码基底

TiLex 是 [pot-app/pot-desktop](https://github.com/pot-app/pot-desktop) 的衍生作品。
原作者 Pylogmon，原项目以 GPL-3.0-only 发布。

分叉自上游 commit `594d32e`（2026-07-04）。自 **2026 年 9 月** 起，
本项目在其基础上做了实质性修改，包括但不限于：

- **移除**：插件系统（`.potext`）、TTS 朗读、生词收藏、二十余家云 OCR 与云翻译服务、
  macOS / Linux 支持
- **新增**：划词 PopButton（低级鼠标钩子）、代码感知的文本预处理、翻译缓存、
  AI 服务预设库与厂商图标体系、模型测速与优选、生词本
- **重构**：OCR 改为独立 sidecar 进程（微信 OCR / Umi-OCR）；
  AI 服务从 `openai` 泛化为 `ai`，支持 OpenAI / Anthropic / Gemini 多种协议
- **更名**：项目更名为 TiLex，bundle identifier 改为 `com.tilex.desktop`

本发布仓库为单提交快照。本项目同样以 **GPL-3.0-only** 发布。

## 设计与思路来源、接口集成

以下项目在产品设计或外部能力集成上给了本项目直接启发与支持：

- [tisfeng/Easydict](https://github.com/tisfeng/Easydict)（GPL-3.0，Tisfeng）
  —— 划词按钮的尺寸与时机、悬浮结果面板的形态、辅助功能取词的行为边界
- [STranslate](https://github.com/STranslate/STranslate)（MIT，zggsong）
  —— 在 Windows 上驱动微信 OCR 的合规隔离思路（解耦主程序二进制 + UI 显式致谢与免责声明）。
  早期版本的 Google Vision OCR 调用方式参考了
  `STranslate.Plugin.Ocr.Google/Main.cs`，该服务其后已整体移除。
- [hiroi-sora/Umi-OCR](https://github.com/hiroi-sora/Umi-OCR)（MIT，HiroiSora）
  —— 本地离线 PaddleOCR 服务的 HTTP API 对接。TiLex 不打包其二进制，
  仅调用用户本机 Umi-OCR 提供的本地服务。

感谢以上项目的作者。

## 图标资源

`app/ui/icons/logo/` 下这 8 个厂商图标取自
[simple-icons](https://github.com/simple-icons/simple-icons) v16.31.0（图标集以 **CC0-1.0** 发布）：

| 文件 | simple-icons slug |
| --- | --- |
| `claude.svg` | `claude` |
| `gemini.svg` | `googlegemini` |
| `qwen.svg` | `qwen` |
| `copilot.svg` | `githubcopilot` |
| `ollama.svg` | `ollama` |
| `kimi.svg` | `kimi` |
| `mimo.svg` | `xiaomi`（MiMo 无专属图标，用小米的厂商 logo）|
| `grok.svg` | `x`（Grok / xAI 无专属图标，用同一家的 X logo）|

图标集本身是 CC0，但**各图标所表示的商标归各自权利人所有**，此处仅作服务识别用途。

## 外部动态库与驱动组件

`app/vendor/wcocr.dll` 编译自
[swigger/wechat-ocr](https://github.com/swigger/wechat-ocr)，编译方式见
`app/vendor/README.md`。该 DLL 独立存放于安装目录，
**不通过 `include_bytes!` 编入主程序二进制**，仅由伴生进程 `tilex-ocr.exe` 加载，
作为与用户本机已安装微信 OCR 引擎的 IPC 通信桥梁。
该组件不含任何文本识别算法或神经网络模型，模型与引擎均来自用户本地微信。

截至本次发布，该上游仓库未声明开源协议。本项目仅用于个人学习与非商用无障碍辅助。
