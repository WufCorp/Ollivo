# Ollivo

[Русский](README.ru.md) · **English**

A desktop app for running open AI models locally on your own computer.
Install it, pick a model, start talking — no Python, Git or CUDA setup.
Text, images, sound and video in one window; your conversations never leave your computer.

Website — [wufcorp.github.io/Ollivo/en/](https://wufcorp.github.io/Ollivo/en/) (source — [site/](site/)), later ollivo.ru.
Windows + NVIDIA, version 0.2 — early. The app speaks English and Russian.

## Download

The latest version is on the [releases](https://github.com/WufCorp/Ollivo/releases/latest) page.
The app doesn't have a Microsoft signature yet, so on first install Windows may show a blue
“Windows protected your PC” window: click “More info”, then “Run anyway”.
The SHA256 checksum of every version is on its release page. After that the app updates itself.

<!-- support:start — Собрано scripts/make-support.mjs из site/support/support.json — правьте там. -->
## Support the project

Ollivo is made by one person. If the app has been useful, support its development:

<a href="https://boosty.to/wufcorp/donate"><img src="site/support/boosty-en.svg" alt="Boosty — a subscription for the project" height="56"></a>&nbsp;&nbsp;<a href="https://yoomoney.ru/to/4100119273215272"><img src="site/support/yoomoney-en.svg" alt="YooMoney — one-time, by card (Russia)" height="56"></a>

Crypto (USDT, TON, Bitcoin) — addresses and QR codes on the **[Support](DONATE.md)** page.
<!-- support:end -->

## What already works

- First-run wizard: checks the computer and the driver, picks a disk, checks the internet
  and a proxy. No administrator rights needed: the app puts the Microsoft libraries
  the engines need right next to them.
- Models — any GGUF, from anywhere: drag a file into the window, and models already downloaded
  in LM Studio, Ollama or ComfyUI are picked up automatically, without copying.
  For those who don't know where to start — a selection of tested models and a HuggingFace search.
- A “traffic light” before downloading: whether the model will run on this computer and how fast —
  in words, compared to reading speed.
- Chat: streaming answers, formatting and code, history and search across conversations, roles and answer style.
  The app picks GPU layers and conversation memory itself; when idle, the model frees the graphics card.
- Documents (PDF, DOCX, ODT, TXT, code), photos for vision models (including WebP and HEIC),
  dictation and transcription of recordings (MP3, M4A, Telegram voice messages, sound from video).
- Project folder: the model reads, searches and edits files in “Manual · Auto · Plan” modes;
  any change can be undone.
- Errors in plain words with buttons, and “Report a problem” in one click.
- Engines install and repair themselves, the app updates itself (updates are signed).
- English and Russian: the language follows Windows and can be changed in the settings.

## What's next

Images via ComfyUI (0.5), one-click tasks (0.6), voice-over, voice mode and video (0.8),
“Pro” mode (0.9). The plan — [docs/plan.md](docs/plan.md) (in Russian).

## How it's built

| Part | Built with |
| --- | --- |
| Shell | Tauri 2 + React + TypeScript |
| Core | Rust: hardware (NVML), downloads, engine manifest, process management |
| Text and vision | llama.cpp (`llama-server`), Vulkan / CUDA 12 / CUDA 13 builds |
| Images and video | ComfyUI + PyTorch, installed on demand |
| Voice | whisper.cpp |

The engine build is chosen by the graphics card: CUDA 13 doesn't work on older cards,
so they get CUDA 12, and chat uses the lightweight Vulkan build by default.

## Building from source

You need Node.js 22+ and stable Rust.

```bash
npm install
npm run tauri dev     # run in development mode
npm run tauri build   # NSIS installer in src-tauri/target/release/bundle/nsis
```

Core tests:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Tests that need the network or real models are marked `#[ignore]` and run separately
with `--ignored`.

Releasing a new version — [RELEASE.md](RELEASE.md).

## Documentation

The project's working documentation is in [docs/](docs/): decisions with reasons, progress
by phase, measurements and pitfalls. A short, complete overview for a newcomer — [HANDOFF.md](HANDOFF.md).
The documentation and code comments are in Russian; the app itself, this README and the website are in both languages.
