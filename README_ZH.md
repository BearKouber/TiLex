# TiLex

Windows 上的划词翻译与截图 OCR 工具。

[English](README.md) | 简体中文

[![License](https://img.shields.io/badge/license-GPL--3.0--only-blue)](LICENSE)
![Platform](https://img.shields.io/badge/platform-Windows%2010%201809%2B-lightgrey)

## 特性

- **划词即翻译** —— 选中文字，旁边浮出一个小按钮，点一下出结果。原文已经是母语时不打扰。
- **截图 OCR** —— 框选屏幕上任意一块认字，识别能力来自本机已装的微信，
  或你自己跑的 [Umi-OCR](https://github.com/hiroi-sora/Umi-OCR)（离线）。
- **六家翻译引擎并排出结果** —— 必应 / 谷歌 / 腾讯交互翻译免配置直接用，
  百度 / DeepL 填自己的 key。
- **任意 OpenAI 兼容的 AI 服务** —— 内置厂商预设，填地址和 key 就能加一个；
  支持 OpenAI / Anthropic / Gemini 三种协议格式。
- **模型测速与优选** —— 一次把账号下的模型全测一遍延迟，直接选最快的那个。
- **生词本** —— 划词结果一键收藏，本地 SQLite 存着。

## 安装

到 [Releases](../../releases) 下载 `.exe` 安装。系统要求 Windows 10 1809 及以上。

截图 OCR 需要本机装过微信并登录过一次（引擎和模型都来自微信，TiLex 不打包它们），
或者自行安装 Umi-OCR 并开启它的 HTTP 服务。

## 使用

默认在托盘运行。

**第一次使用**：翻译设置 → 划词浮标 → 悬停。服务设置 → 添加服务。文本识别 OCR 可根据情况添加。

**配一个 AI 服务**：服务设置 → 添加服务 → 添加 AI 服务 → 选一个厂商预设（或自定义）→ 填 API 地址和密钥 → 点「获取模型列表」→ 选模型 → 保存。

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
