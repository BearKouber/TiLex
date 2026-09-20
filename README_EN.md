<p align="center">
  <img src="ui/icons/app.png" width="112" height="112" alt="TiLex" />
</p>

<h1 align="center">TiLex</h1>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-GPL--3.0--only-blue" alt="License: GPL-3.0-only" /></a>
</p>

<p align="center">A Windows translation and screenshot OCR tool with selection popups. Native UI built with Rust + Slint, no browser engine bundled.</p>

<p align="center"><a href="README.md">简体中文</a> | English</p>

## Interface Preview

<details>
<summary>Click to expand</summary>

<p align="center">
  <img width="800" src="https://github.com/user-attachments/assets/4d5b7140-f4a8-4f0a-8dc9-056d175b345d" alt="Translate" />
</p>
<p align="center">
  <img width="800" src="https://github.com/user-attachments/assets/d438a123-e919-4cea-9ec4-241c92ec13fe" alt="Translate Service Settings" />
</p>
<p align="center">
  <img width="800" src="https://github.com/user-attachments/assets/920b8015-d6f7-4b76-bc37-ecd10d2f39c3" alt="OCR Service Settings" />
</p>
<p align="center">
  <img width="800" src="https://github.com/user-attachments/assets/305dd3d1-8b03-40bd-a495-7a012853b483" alt="Wordbook" />
</p>
<p align="center">
  <img width="800" src="https://github.com/user-attachments/assets/5d783cac-1714-48be-84a5-ca441aff9964" alt="About" />
</p>
<p align="center">
  <img width="320" src="https://github.com/user-attachments/assets/7d500be1-49ff-4e4e-b4a0-bcbf2e139906" alt="Selection PopWindow" />
</p>

</details>

## Features

- **Select to Translate** — Select text to bring up a floating button; click to see results. Silent when the selection is already in your native language. Close the popup with Esc, pin it to keep it open, or drag it by the top bar.
- **Screenshot OCR** — Select any area on the screen to recognize text. Powered by local WeChat OCR (if installed) or offline [Umi-OCR](https://github.com/hiroi-sora/Umi-OCR).
- **Multi-Engine Results** — Bing, Google, and Tencent Transmart work out of the box with no API keys. Baidu and DeepL can be used with your own keys, and you can add any AI service.
- **Any OpenAI-Compatible AI Service** — Built-in vendor presets. Just enter the URL and API key. Supports OpenAI, Anthropic, and Gemini protocol formats. One "Custom instructions" box sets the translation style; words and sentences can come with examples and usage notes.
- **Model Speed Test & Selection** — Benchmark latency across all available models on your account in one run to pick the fastest one.
- **Wordbook** — Save words from translation results into a local SQLite database with one click. Supports multi-select delete and Markdown export.

## Installation

Download the installer (`.exe`) from [Releases](../../releases). Requires Windows 10 1809 or higher.

Screenshot OCR requires WeChat installed and logged in at least once (engines and models come from WeChat; TiLex does not bundle them), or install Umi-OCR and start its HTTP service.

## Usage

Runs in the system tray by default.

**First Time Setup**: Translation Settings → Floating Icon → Hover. Service Settings → Add Service. Add OCR service if needed.

**Configure an AI Service**: Service Settings → Add Service → Add AI Service → choose a preset (or Custom) → enter API URL and key → click "Fetch Models" → select model → Save.

**VS Code Selection Issue**: VS Code does not expose text selections to the system by default. Set `editor.accessibilitySupport` to `on` in VS Code settings.

## Build from Source

Requires a stable Rust toolchain and MSVC build tools. No Node, no frontend build step.

```bash
cargo build --release -p tilex -p tilex-ocr
```

Output lands in `target/release/`: `tilex.exe` and its companion `tilex-ocr.exe`.
Together with `vendor/wcocr.dll` from the source tree they run as-is, no install needed.

To produce an installer, run `cargo install cargo-packager` once, then:

```bash
cargo packager --release
```

It only packages, it does not compile, so the `cargo build` above must have run first.
The output is `target/release/tilex_<version>_x64-setup.exe`.

## Community

Thanks to the [LinuxDO](https://linux.do/) community for their support.

## License

TiLex is licensed under [GPL-3.0-only](LICENSE).

This project is a derivative work of [pot-app/pot-desktop](https://github.com/pot-app/pot-desktop) (Pylogmon, GPL-3.0-only) with substantial modifications.
Product design is also inspired by [Easydict](https://github.com/tisfeng/Easydict) and [STranslate](https://github.com/STranslate/STranslate).
See [NOTICE.md](NOTICE.md) for full origin and modification details.

This program comes with no warranty.
