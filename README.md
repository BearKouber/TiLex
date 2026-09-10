# TiLex

A Windows translation and screenshot OCR tool with selection popups.

English | [简体中文](README_ZH.md)

[![License](https://img.shields.io/badge/license-GPL--3.0--only-blue)](LICENSE)
![Platform](https://img.shields.io/badge/platform-Windows%2010%201809%2B-lightgrey)

## Features

- **Select to Translate** — Select text to bring up a floating button; click to see results. Silent when the selection is already in your native language.
- **Screenshot OCR** — Select any area on the screen to recognize text. Powered by local WeChat OCR (if installed) or offline [Umi-OCR](https://github.com/hiroi-sora/Umi-OCR).
- **Multi-Engine Results** — Bing, Google, and Tencent Transmart work out of the box with no API keys. Baidu and DeepL can be used with your own keys.
- **Any OpenAI-Compatible AI Service** — Built-in vendor presets. Just enter the URL and API key. Supports OpenAI, Anthropic, and Gemini protocol formats.
- **Model Speed Test & Selection** — Benchmark latency across all available models on your account in one run to pick the fastest one.
- **Wordbook** — Save words from translation results into a local SQLite database with one click.

## Installation

Download the installer (`.exe`) from [Releases](../../releases). Requires Windows 10 1809 or higher.

Screenshot OCR requires WeChat installed and logged in at least once (engines and models come from WeChat; TiLex does not bundle them), or install Umi-OCR and start its HTTP service.

## Usage

Runs in the system tray by default.

**First Time Setup**: Translation Settings → Selection PopButton → Hover. Service Settings → Add Service. Add OCR service if needed.

**Configure an AI Service**: Service Settings → Add Service → Add AI Service → choose a preset (or Custom) → enter API URL and key → click "Fetch Model List" → select model → Save.

**VS Code Selection Issue**: VS Code does not expose text selections to the system by default. Set `editor.accessibilitySupport` to `on` in VS Code settings.

## Build from Source

Requires Node 21, pnpm, Rust toolchain, and MSVC build tools.

```bash
pnpm install
npx tauri build                  # Full installer bundle
npx tauri build --bundles none   # Compile exe only, for testing
```

`beforeBuildCommand` builds the OCR sidecar first (`pnpm build:sidecar`), which is an independent crate in `src-tauri/ocr-sidecar/`.

## Community

Thanks to the [LinuxDO](https://linux.do/) community for their support.

## License

TiLex is licensed under [GPL-3.0-only](LICENSE).

This project is a derivative work of [pot-app/pot-desktop](https://github.com/pot-app/pot-desktop) (Pylogmon, GPL-3.0-only) with substantial modifications.
Product design is also inspired by [Easydict](https://github.com/tisfeng/Easydict) and [STranslate](https://github.com/STranslate/STranslate).
See [NOTICE.md](NOTICE.md) for full origin and modification details.

This program comes with no warranty.
