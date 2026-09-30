# Reflow

Local-first desktop dictation with a focused workspace for your words.

**Version 0.2.0** redesigns the desktop interface and strengthens recording ownership, settings persistence, phone pairing, and clipboard safety. Read the [release notes](docs/release-0.2.0.md) and [verification record](docs/release-verification-0.2.0.md).

## ✨ Features & Architecture

- **🔒 Offline & Local-First**: Recognition and cleanup run locally. Initial model and runtime downloads require internet access; audio is sent only to local runtimes.
- **⚡ Real Qwen3-ASR on your GPU**:
  - Pick **0.6B** for lower latency or **1.7B** for the larger model. When the requested model exceeds the GPU memory budget, Reflow chooses a smaller installed model that fits and reports the downgrade. CPU is used when GPU acceleration is unavailable or no installed model fits.
  - The default Python runtime needs system Python, torch, transformers and torchao; these are not bundled. Installed `-hf` weights load automatically when Keep loaded is enabled (downloads: ~1.56 / ~4.08 GB).
  - Audio is buffered continuously and transcribed on release. Live partials are disabled (`LIVE_PARTIALS = False`). Long dictations use silence-aligned segments of at most 30 seconds, with a one-hour recording limit and visible truncation warnings.
  - Custom dictionary terms are passed to the model as recognition hotwords.
- **🔒 Single instance**: launching Reflow twice focuses the running app instead of fighting over the global hotkey.
- **🌊 Voice Activity Detection (VAD)**:
  - RMS energy detection drives the audio-level display and configurable silence auto-stop. Capture forwards all audio rather than discarding non-speech frames.
  - The ASR runtime trims surrounding silence before decoding.
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
  - Single item copy, original/cleaned comparison, deletion, and batch clear with confirmation.
- **🖥️ Minimalist HUD & System Tray Utility**:
  - Borderless, semi-transparent floating overlay with a live waveform visualizer; the transcript appears when recognition finishes.
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
│   │   └── globals.css               # Tailwind 4 & shared workspace styles
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
- **Python runtime**: Python 3.11+ on PATH with the exact packages in [model-runtime/requirements.txt](model-runtime/requirements.txt); tested on Python 3.11.9. NVIDIA acceleration needs a compatible CUDA torch build. Python is not required for the experimental native runtime.
- **OS**: Windows 10/11 or Linux (X11 full support; Wayland best-effort)
- An NVIDIA GPU is recommended for Python ASR; it otherwise uses the CPU. Model and precision selection depend on measured free memory.

### ASR runtime setup

Python is the default and remains the fallback option. Install its pinned requirements into the Python interpreter Reflow finds on PATH:

```bash
python -m pip install -r model-runtime/requirements.txt
```

For NVIDIA acceleration, install the matching torch wheel first. The tested Windows setup used torch 2.13.0+cu130 from the [official CUDA 13.0 wheel index](https://download.pytorch.org/whl/cu130/torch/):

```bash
python -m pip install torch==2.13.0 --index-url https://download.pytorch.org/whl/cu130
python -m pip install -r model-runtime/requirements.txt
```

Select **Settings → Model → Speech runtime → Native (experimental)** to try Python-free ASR. Download its separate Q8_0 decoder and audio-projector files (~1.02 GB for 0.6B or ~2.52 GB for 1.7B) and the llama.cpp runtime through Model settings. It reuses the refinement server lifecycle while running a second local server. Native audio is decoded in silence-aligned segments of at most eight seconds; custom dictionary context and token-cap warnings are preserved. The Python sidecar remains available by switching the setting back.

The downloader pins llama.cpp stable **v0.5.0 / b11146**, with checksums for each supported archive. Model downloads use pinned Hugging Face revisions and verify weight SHA-256 before completing. See [runtime-pins.json](docs/runtime-pins.json) for artifact provenance. Native live recognition and its English/Hindi accuracy gate remain unverified, so the default is **Python**; see the [verification report](docs/runtime-fix-report.md).

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
cargo test --manifest-path src-tauri/Cargo.toml
```

### First run

The first launch downloads nothing automatically. Set up Python as above, or select the experimental native runtime, then open **Settings → Model → Download model**. Files are stored in `%APPDATA%\reflow\models` on Windows. With Keep loaded enabled, an installed model loads at startup on the selected available backend. To regenerate the app icon set: `python scripts/generate_icons.py`. To verify the Python ASR pipeline headlessly: `python scripts/test_sidecar.py path/to/speech.wav` (expects an existing recording).

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

Linux packages produced by `npm run tauri build`: `.deb` and AppImage. Runtime: system `python3` plus the pinned requirements for the default Qwen sidecar (not bundled inside the AppImage), or the separately downloaded experimental native runtime and its models. Pushing a version tag such as `v0.1.0` runs the GitHub Actions release workflow and publishes Linux, Windows, and macOS installers.

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
cargo run --manifest-path src-tauri/Cargo.toml -- --api --bind 127.0.0.1:7840

# Display hardware and active ASR status
cargo run --manifest-path src-tauri/Cargo.toml -- --status

# Show recorded dictation latency and hardware metrics
cargo run --manifest-path src-tauri/Cargo.toml -- --benchmark

# Compare real audio across both ASR sizes and runtimes
cargo run --manifest-path src-tauri/Cargo.toml -- --benchmark path/to/corpus.json

# List local SQLite history entries
cargo run --manifest-path src-tauri/Cargo.toml -- --history-list
```

The audio benchmark takes a JSON array of `{ "language": "en", "audio": "sample.wav", "reference": "spoken text" }`. WAV paths are relative to the corpus file; use 16 kHz mono PCM16. Supply English and Hindi references and install both sizes for both runtimes to measure the accuracy gate. Missing inputs remain explicit errors, and a warning about truncated decoding invalidates that measurement. The report includes WER, mean decode latency and driver-reported resident VRAM delta; it does not change settings automatically.

---

## 🧪 Testing

Run the full automated test suite:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
python model-runtime/qwen3_asr_runtime.py --selftest
npm run lint
npm run test
npm run build
```

Tests cover:

- Audio resampling (48kHz/44.1kHz → 16kHz mono)
- Voice Activity Detection (RMS energy, pre/post roll buffers, silence auto-stop)
- Transcript stabilization (prefix commitment & mutable suffix)
- Filler word & stutter removal
- Spoken punctuation & sentence capitalization
- Custom dictionary & regex replacements
- Language settings and Unicode WER scoring; real English/Hindi recognition requires an audio corpus and installed weights
- SQLite history persistence, search, batch deletion, and retention policy
- Settings JSON persistence and atomic updates

---

## 📄 License

MIT License — free and open source.
