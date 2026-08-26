# Reflow 🎙️

> **Production-quality, 100% local Wispr Flow-style dictation for Windows, Linux, and Android.**  
> Desktop is **Rust** (Tauri 2 + in-process **Dory** realtime dataflow). Android is a **Kotlin** companion that streams audio to the desktop. ASR is **Qwen3-ASR-0.6B** on the computer — never in the cloud.

---

## ✨ Features & Architecture

- **🔒 100% Offline & Local-First**: No audio is ever transmitted across the internet. Zero cloud dependency.
- **⚡ Real Qwen3-ASR on your GPU**:
  - Pick your model in Settings: **0.6B** (realtime, fits any GPU) or **1.7B** (max accuracy — state of the art among open ASR models; needs ~6 GB VRAM, falls back to CPU if it doesn't fit).
  - The model loads automatically at startup — CUDA first, CPU fallback (transformers ≥ 5.13, `-hf` weights, ~1.6 / ~3.5 GB).
  - Continuous audio streaming (no VAD gating), streaming partials while you speak, full transcription on release.
  - Custom dictionary terms are passed to the model as recognition hotwords.
- **🔒 Single instance**: launching Reflow twice focuses the running app instead of fighting over the global hotkey.
- **🌊 Voice Activity Detection (VAD)**:
  - Low-latency RMS energy thresholding with pre-roll (300ms) and post-roll hangover (500ms) to ensure zero phoneme clipping.
  - Configurable auto-stop silence duration.
- **⌨️ Push-to-talk any way you like**: regular combos (Ctrl+Space) via the OS hotkey API, or **modifier-only combos like Shift+Win** via a low-level keyboard hook — hold to record (audio is buffered silently), release to transcribe and insert. The recorder in Settings captures any combination.
- **📋 Native Atomic Text Injection**:
  - High-speed clipboard injection with automatic user clipboard save & safe restoration.
  - Active window and process detection for context-aware profiles.
- **✨ Intelligent Formatting & Cleanup**:
  - Filler word removal ("um", "uh", "er", "ah", "hmm").
  - Immediate stuttering word deduplication.
  - Spoken punctuation parsing ("period", "comma", "question mark", "new line").
  - Custom vocabulary & replacement dictionary (e.g. `git hub` → `GitHub`, `vs code` → `VS Code`, `tauri` → `Tauri`).
  - Context profiles: **Normal**, **Coding** (preserves camelCase, commands, syntax), **Email**, **Chat**, and **Notes**.
- **💾 Local SQLite History & Search**:
  - Searchable full-text transcription history.
  - Configurable data retention policies (1 day, 7 days, 30 days, 90 days, Forever).
  - Single item copy, re-inject, deletion, and batch clear with confirmation.
- **🖥️ Minimalist HUD & System Tray Utility**:
  - Borderless, semi-transparent floating overlay with live waveform visualizer and transcript stabilization (committed prefix + mutable suffix).
  - Native system tray with status, language toggle, history, and settings shortcuts.
- **📊 Real-Time Developer Diagnostics**:
  - Latency waterfall breakdown charts.
  - Live CPU %, App RAM, Model VRAM, and internal ASR event stream.

---

## 🏗️ Project Structure

```text
reflow/
├── package.json
├── tsconfig.json
├── vite.config.ts
├── src/
│   ├── components/
│   │   ├── DictateHome.tsx           # Live dictation studio & test page
│   │   ├── HistoryView.tsx           # Searchable SQLite history manager
│   │   ├── HotkeyPicker.tsx          # Reusable global-shortcut recorder
│   │   ├── Navigation.tsx            # Sidebar nav with model badge & quit
│   │   ├── Onboarding.tsx            # First-run wizard
│   │   ├── Overlay.tsx               # Floating recording HUD overlay
│   │   ├── OverlayApp.tsx            # Overlay window root (uses ?window=overlay)
│   │   ├── SettingsView.tsx          # Settings shell with searchable sidebar
│   │   ├── TitleBar.tsx              # Custom title bar with status pill
│   │   ├── Waveform.tsx              # Real-time audio waveform visualizer
│   │   └── settings/
│   │       ├── ui.tsx                # Shared Section / Row / Toggle
│   │       ├── GeneralPage.tsx
│   │       ├── AudioPage.tsx
│   │       ├── ModelPage.tsx
│   │       ├── CleanupPage.tsx
│   │       ├── DictionaryPage.tsx
│   │       ├── PhonePage.tsx
│   │       └── AdvancedPage.tsx
│   ├── services/
│   │   └── tauriApi.ts               # Strongly typed Tauri IPC bridge
│   ├── styles/
│   │   └── globals.css               # Tailwind 4 & glassmorphism styling
│   ├── types/
│   │   └── index.ts                  # Shared TypeScript interfaces
│   ├── historyDisplay.ts             # History entry text/undo helpers
│   ├── App.tsx                       # Main application view
│   └── main.tsx                      # Root entry point
├── src-tauri/
│   ├── Cargo.toml                    # Native dependencies (cpal, rusqlite, arboard, sysinfo, etc.)
│   ├── tauri.conf.json               # Tauri v2 configuration & window properties
│   ├── capabilities/
│   │   └── default.json              # Tauri 2 security permissions
│   ├── src/
│   │   ├── audio/                    # Native cpal capture (WASAPI / ALSA / PipeWire), VAD, resampling
│   │   ├── asr/                      # ASREngine trait, Qwen3-ASR sidecar, stabilizer, mock engine
│   │   ├── formatting/               # Cleaner, punctuation inferer, replacements, context modes
│   │   ├── injection/                # Clipboard paste + platform paste chords
│   │   ├── history/                  # SQLite store, search, retention cleaner
│   │   ├── hotkey/                   # Global hotkey manager (Push-to-Talk / Toggle)
│   │   ├── model/                    # Model directory, checksum verification, disk stats
│   │   ├── dory/                     # In-process Dory realtime dataflow bus (desktop)
│   ├── session.rs                # Shared dictation session (hotkey + Android API)
│   ├── api/                      # LAN HTTP + WebSocket API for Android
│   ├── pairing.rs                # Pairing codes and hashed device tokens
│   ├── platform/                 # Windows / Linux / macOS adapters, paths, diagnostics
│   │   ├── settings/                 # JSON configuration store
│   │   ├── state.rs                  # Synchronized AppState and LatencyTracker
│   │   ├── commands/                 # All Tauri IPC command handlers
│   │   ├── lib.rs                    # Tauri app lifecycle & system tray
│   │   └── main.rs                   # App entrypoint & CLI commands
│   └── tests/
│       └── integration_tests.rs      # Comprehensive automated backend test suite
├── model-runtime/
│   └── qwen3_asr_runtime.py          # Standalone offline Qwen3-ASR Python IPC sidecar
├── android/                          # Kotlin + Compose companion (LAN client)
├── linux/                            # .desktop, AppStream, systemd user unit
├── docs/                             # LAN API + OpenAPI
└── README.md
```

---

## 🚀 Getting Started

### Prerequisites

- **Node.js**: v18+ (tested on Node v24)
- **Rust**: 1.77+ (tested on Rust 1.97)
- **Python**: 3.10+ with `torch` (CUDA build for GPU), `transformers >= 5.13`, `huggingface_hub`, and `numpy`
- **OS**: Windows 10/11 or Linux (X11 full support; Wayland best-effort)
- CUDA-capable GPU recommended (a 4 GB card runs the 0.6B model in bf16); CPU works

### Installation

#### Installers

Prebuilt installers are published in [GitHub Releases](https://github.com/Lawliet2004/reflow/releases):

- **Windows**: download the `.msi` or NSIS `.exe` installer.
- **macOS**: download the `.dmg` matching your Mac (Intel or Apple Silicon).
- **Linux**: download the `.deb` package on Debian/Ubuntu, or the `.AppImage` on other distributions.

The app is unsigned while the project is being developed, so macOS and Windows may show an unverified-developer warning on first launch.

```bash
# 1. Install frontend dependencies
npm install

# 2. Build and verify test suites
npm run build
cd src-tauri && cargo test
```

### First run

The first launch downloads nothing automatically — open **Settings → Model → Download model** once (~1.6 GB from Hugging Face into `%APPDATA%\reflow\models`). After that the model loads on your GPU every time the app starts. To regenerate the app icon set: `python scripts/generate_icons.py`. To verify the ASR pipeline headlessly: `python scripts/test_sidecar.py` (generates spoken audio via Windows SAPI).

### Linux packages (Debian/Ubuntu)

Build-time:

```bash
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev libasound2-dev libxdo-dev libssl-dev patchelf pkg-config
```

Runtime: PipeWire or PulseAudio (ALSA via `pipewire-alsa` is enough for `cpal`). The `.deb` also depends on WebKitGTK, GTK 3, Ayatana AppIndicator, and `libxdo3`.

Linux notes:

- Default hotkey is **Ctrl+Shift+Space** (Ctrl+Space is often taken by IBus/fcitx).
- **X11**: global hotkey, overlay, and clipboard paste work. Terminals use **Ctrl+Shift+V**.
- **Wayland**: audio, tray, and history work. Global shortcuts and key injection are compositor-limited. If paste cannot be simulated, the transcript stays on the clipboard and the overlay says `Copied — press Ctrl+V`.
- Data directory: `~/.local/share/reflow`
- Autostart: `~/.config/autostart/reflow.desktop`

### Running Reflow Desktop Application

```bash
# Launch Reflow in development mode
npm run tauri dev
```

Linux packages produced by `npm run tauri build`: `.deb` and AppImage. Runtime: `python3` for the Qwen sidecar (not bundled inside the AppImage). Pushing a version tag such as `v0.1.0` runs the GitHub Actions release workflow and publishes Linux, Windows, and macOS installers.

### Android companion

The phone is a remote microphone. Enable **Settings → Android / LAN API** on the desktop, then pair with the 6-digit code.

```bash
# Headless API (Linux user systemd unit: linux/reflow-api.service)
reflow --api --bind 0.0.0.0:7840
```

Open the `android/` folder in Android Studio to build the APK. See [docs/android-api.md](docs/android-api.md).

---

## 🛠️ CLI Utilities

Reflow includes built-in command-line tools for diagnostics, benchmarks, and automation:

```bash
# Headless LAN API for Android
cargo run -- --api --bind 127.0.0.1:7840

# Display hardware and active ASR status
cargo run -- --status

# Run latency and hardware benchmark
cargo run -- --benchmark

# List local SQLite history entries
cargo run -- --history-list
```

---

## 🧪 Testing

Run the full automated test suite:

```bash
cargo test
```

Tests cover:
- Audio resampling (48kHz/44.1kHz → 16kHz mono)
- Voice Activity Detection (RMS energy, pre/post roll buffers, silence auto-stop)
- Transcript stabilization (prefix commitment & mutable suffix)
- Filler word & stutter removal
- Spoken punctuation & sentence capitalization
- Custom dictionary & regex replacements
- Mixed language recognition (English, Hindi, Bengali)
- SQLite history persistence, search, batch deletion, and retention policy
- Settings JSON persistence and atomic updates

---

## 📄 License

MIT License — free and open source.
