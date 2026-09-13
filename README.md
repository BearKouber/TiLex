<p align="center">
  <img src="src-tauri/icons/128x128.png" width="112" height="112" alt="TiLex" />
</p>

<h1 align="center">TiLex</h1>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0--only-blue" alt="License: GPL-3.0-only" /></a>
</p>

<p align="center">Windows 上的划词翻译与截图 OCR 工具。</p>

<p align="center">简体中文 | <a href="README_EN.md">English</a></p>

## 界面预览

<details>
<summary>点击展开</summary>

<p align="center">
  <img width="800" src="https://github.com/user-attachments/assets/c184bb95-a677-42f6-91ec-716480de1be0" alt="翻译界面" />
</p>
<p align="center">
  <img width="800" src="https://github.com/user-attachments/assets/4d975e7d-6209-46ed-81fb-a8e31f561bde" alt="翻译服务设置" />
</p>
<p align="center">
  <img width="800" src="https://github.com/user-attachments/assets/91a37bc0-3ecc-4967-a1cc-456fc3411800" alt="OCR 服务设置" />
</p>
<p align="center">
  <img width="800" src="https://github.com/user-attachments/assets/98799949-97ea-46f7-b520-5ffc92c47b6a" alt="生词本" />
</p>
<p align="center">
  <img width="800" src="https://github.com/user-attachments/assets/81bc6e7c-de4c-4808-a19b-3f6a85724a27" alt="关于" />
</p>
<p align="center">
  <img width="320" src="https://github.com/user-attachments/assets/ec1a3e2a-4531-4a94-9244-1507a12bd199" alt="划词弹窗" />
</p>

</details>

## 特性

- **划词即翻译** —— 选中文字，旁边浮出一个小按钮，点一下出结果。原文已经是母语时不打扰。
  浮窗可按 Esc 关闭、钉住不自动隐藏、按住顶栏拖动。
- **截图 OCR** —— 框选屏幕上任意一块认字，识别能力来自本机已装的微信，
  或你自己跑的 [Umi-OCR](https://github.com/hiroi-sora/Umi-OCR)（离线）。
- **多家翻译引擎并排出结果** —— 必应 / 谷歌 / 腾讯交互翻译免配置直接用，
  百度 / DeepL 填自己的 key，还可以加任意 AI 服务。
- **任意 OpenAI 兼容的 AI 服务** —— 内置厂商预设，填地址和 key 就能加一个；
  支持 OpenAI / Anthropic / Gemini 三种协议格式。
  用一段「自定义要求」控制翻译风格，单词和句子都能附带例句和用法说明。
- **模型测速与优选** —— 一次把账号下的模型全测一遍延迟，直接选最快的那个。
- **生词本** —— 划词结果一键收藏，本地 SQLite 存着；支持多选删除和导出 Markdown。

## 安装

到 [Releases](../../releases) 下载 `.exe` 安装。系统要求 Windows 10 1809 及以上。

截图 OCR 需要本机装过微信并登录过一次（引擎和模型都来自微信，TiLex 不打包它们），
或者自行安装 Umi-OCR 并开启它的 HTTP 服务。

## 使用

默认在托盘运行。

**第一次使用**：翻译设置 → 划词浮标 → 悬停。服务设置 → 添加服务。文本识别 OCR 可根据情况添加。

**配一个 AI 服务**：服务设置 → 添加服务 → 添加 AI 服务 → 选一个厂商预设（或自定义）→ 填 API 地址和密钥 → 点「拉取模型列表」→ 选模型 → 保存。

**VS Code 内划词没反应**：VS Code 默认不向系统暴露选区。在设置里把 `editor.accessibilitySupport` 改成 `on` 即可。

## 从源码构建

需要 Node 21、pnpm、Rust 工具链、MSVC 生成工具。

```bash
pnpm install
npx tauri build                  # 完整打包
npx tauri build --bundles none   # 只编译不打包，验证能过就够
```

`beforeBuildCommand` 会先编 OCR sidecar（`pnpm build:sidecar`），
它是 `src-tauri/ocr-sidecar/` 下的独立 crate。

## 社区

感谢 [LinuxDO](https://linux.do/) 社区的支持。

## 开源协议

TiLex 以 [GPL-3.0-only](LICENSE) 发布。

本项目是 [pot-app/pot-desktop](https://github.com/pot-app/pot-desktop)（Pylogmon，GPL-3.0-only）的衍生作品，已做实质性修改。
设计上另受 [Easydict](https://github.com/tisfeng/Easydict) 与 [STranslate](https://github.com/STranslate/STranslate) 启发。
完整来源与修改说明见 [NOTICE.md](NOTICE.md)。

本程序不提供任何担保。
