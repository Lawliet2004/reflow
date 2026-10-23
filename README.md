<p align="center"><img src="docs/assets/reflow-lockup.png" alt="Reflow" width="420"></p>

Local-first desktop dictation with a focused workspace for your words.

**Version 0.4.0** adds Zipformer speech recognition, automatic dictionary learning, and task-based writing cleanup. This release also hardens startup privacy recovery, model-runtime activation, and audio session ownership. Read the [release notes](docs/release-0.4.0.md) and [backend audit](docs/backend-audit-0.4.0.md).

## ✨ Features & Architecture

- **Spoken snippets and custom modes**: exact text expansion, optional Jira/commit/medical presets, per-mode shortcuts, app/spoken triggers, local context permissions, output translation and six output destinations.
- **Command, assistant and notes**: voice-edit selections; ask a stateless local assistant; capture manual or voice notes with search, pins and Markdown export. Correct history transcripts and review vocabulary suggestions.
- **Local audio imports**: WAV, MP3, M4A/AAC, FLAC and Ogg, up to two hours. Cancellable transcription and TXT/SRT/VTT exports use actual audio segment offsets. Speaker metadata remains optional.
- **Privacy controls**: airplane mode blocks artifact downloads; a local network ledger records outbound hosts/bytes. Optional OS-keyring AES-256-GCM encryption covers transcripts, selected command text, subtitle segments and retained audio; missing keys preserve the original database. Encrypted searches decrypt bounded pages.
- **Desktop options**: usage statistics, pins/tags/filters, retry options, config transfer, offline model imports, sound cues, app exclusions, media ducking on Windows, battery policy, accessible HUD contrast, paste delay and type insertion.
- **Meetings and assistant actions**: optional mic/system-audio recording, chunked local transcript summaries, and explicitly activated web-search/note proposals. See [meeting and assistant guide](docs/meeting-and-assistant.md).
- **Local automation and portable mode**: revocable localhost token, authenticated dictation/session routes, `--dictate` and `--portable`. See [automation documentation](docs/local-automation.md), [offline transfer](docs/offline-transfer.md), and [expansion verification](docs/feature-expansion-verification.md).

- **🔒 Offline & Local-First**: Recognition and cleanup run locally. Initial model and runtime downloads require internet access; audio is sent only to local runtimes.
- **⚡ Real Qwen3-ASR on your GPU**:
  - Pick **0.6B** for lower latency or **1.7B** for the larger model. When the requested model exceeds the GPU memory budget, Reflow chooses a smaller installed model that fits and reports the downgrade. CPU is used when GPU acceleration is unavailable or no installed model fits.
  - The default Python runtime needs system Python, torch, transformers and torchao; these are not bundled. Installed `-hf` weights load automatically when Keep loaded is enabled (downloads: ~1.56 / ~4.08 GB).
  - Audio is buffered continuously and transcribed on release. Live partials are disabled (`LIVE_PARTIALS = False`). Long dictations use silence-aligned segments of at most 30 seconds, with a one-hour recording limit and visible truncation warnings.
  - Custom dictionary terms are passed to the model as recognition hotwords.
- **🔒 Single instance**: launching Reflow twice focuses the running app instead of fighting over the global hotkey.
- **⚡ Phonon-2 for English**: an optional 164 MB model download with optimized CPU kernels and NVIDIA CUDA execution. Select it under Settings → Performance. English-only; memory use exceeds the compressed download size. See [setup and verified CPU/GPU results](docs/phonon-2.md).
- **Session foundations**: additive settings-v5 and history-v6 migrations preserve existing data, with independent session intents, mode and output contracts, and multiple shortcut bindings. Escape or the recording Cancel button discards the active session.
- **🌊 Voice Activity Detection (VAD)**:
  - RMS energy detection drives the audio-level display and configurable silence auto-stop. Capture forwards all audio rather than discarding non-speech frames.
  - The ASR runtime trims surrounding silence before decoding.
- **⌨️ Push-to-talk**: hold a shortcut to record, then release to transcribe and insert. Double-press within half a second to keep recording hands-free, then press the shortcut again to stop. For multi-key shortcuts, keep the modifiers held and double-tap the remaining key. Quick single taps wait for the double-tap window before stopping. Regular combinations use the OS hotkey API on Windows and X11, or the desktop's GlobalShortcuts portal on Wayland. **Modifier-only combinations like Ctrl+Shift, Ctrl+Win and Shift+Win** work through the keyboard hook on Windows. On KDE Plasma 6.1+ Wayland, select them in Reflow; they use tap-to-start/tap-to-stop because KDE activates them on release. Other Linux desktops still require a regular key.
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
  - Separate audio retention in Settings → Advanced: don't save audio (default), 1 day, 1 week, 1 month (30 days), or Maximum (indefinitely). Expiry is measured from recording completion. Expired recordings are deleted while the app runs and at startup; Extract audio and Retry transcript disappear at expiry. Transcripts follow their own retention policy.
  - Extract audio saves a WAV in Downloads. Retry transcript recognizes the saved recording again and applies the current cleanup settings. Deleting history also deletes its audio. Original/cleaned views remain available after audio expiry; no Undo AI edit action is shown.
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

- **Node.js**: v22+ (tested on Node v24)
- **Rust**: 1.88+ (tested on Rust 1.97)
- **Python runtime**: Python 3.11+ with the exact packages in [model-runtime/requirements.txt](model-runtime/requirements.txt); tested on Python 3.11.9 on Windows and Python 3.14 on Linux. Development builds discover the checkout's `.venv` automatically. NVIDIA acceleration needs a compatible CUDA torch build. Python is not required for the experimental native runtime.
- **OS**: Windows 10/11 or Linux (X11 full support; Wayland best-effort)
- An NVIDIA GPU is recommended for Python ASR; it otherwise uses the CPU. Model and precision selection depend on measured free memory.

### ASR runtime setup

For a new installation, open the app and follow **Microphone → Shortcut → Speech model**. The speech step checks the operating system, processor, memory, graphics and free storage. Click **Download all dependencies** to install Python, an isolated environment and speech libraries. Then choose and download a speech model. **Download and set up** also runs both steps together. Neither path needs a terminal or system Python.

Compatible NVIDIA GPUs get matching CUDA libraries automatically: CUDA 13 for Turing or newer with a supporting driver, or CUDA 12.6 for supported older cards/drivers. Setup runs a GPU operation to verify acceleration. If compatibility cannot be established, it chooses CPU packages and explains why. AMD graphics are detected, but automatic AMD acceleration is not yet supported by the Python speech runtime; CPU dictation remains available. Intel Macs are offered Zipformer. Processor names and instruction support are reported directly rather than guessing a processor's age or promising performance.

Setup shows a conservative peak-storage budget for the selected engine/device and model, including the replacement environment, package cache, extraction, temporary files and model staging. Zipformer installs a torch-free package set; CPU preferences install CPU packages even on NVIDIA systems. Setup supports cancellation and retry and reuses completed cached downloads. Replacement environments are verified before activation, preserving the earlier working generation on failure. Airplane mode requires enabling downloads first. Your selected custom interpreter is preserved; clear it in Model settings to use the managed environment.

Python is the default and remains the fallback option. On first launch, choose a speech model and click **Download and set up** (or **Allow downloads and set up** in Airplane mode). Reflow downloads a managed Python interpreter and speech packages, then downloads the selected model automatically. Progress and cancellation are available during runtime installation; a failed setup can be retried. The environment persists across app launches. This works for both development launches and packaged installations.

For development, you can alternatively create a virtual environment in the checkout and install the pinned requirements there. On Debian/Ubuntu, install `python3-venv` with apt first if the venv command is unavailable; system Python is managed by the operating system.

```bash
python3 -m venv .venv
.venv/bin/python -m pip install -r model-runtime/requirements.txt
npm run tauri dev
```

Binaries inside the checkout, including local release builds, discover its `.venv` automatically; activation is optional. A configured user runtime can take precedence. The verified managed environment takes precedence over an inherited `VIRTUAL_ENV`, and `REFLOW_PYTHON=/absolute/path/to/python` explicitly selects an interpreter for either development or installed builds. On Windows, the checkout environment's interpreter is `.venv\Scripts\python.exe`.

For an existing Windows Python installation, install into the interpreter Reflow finds on PATH:

```bash
python -m pip install -r model-runtime/requirements.txt
```

The requirements include the pinned Phonon runtime. Existing installations can add it with `python -m pip install -r model-runtime/requirements-phonon.txt`. Phonon-2 is an optional English model in the catalog; see its [integration guide](docs/phonon-2.md).

For NVIDIA acceleration, install the matching torch wheel first. The tested Windows setup used torch 2.13.0+cu130 from the [official CUDA 13.0 wheel index](https://download.pytorch.org/whl/cu130/torch/):

```bash
python -m pip install torch==2.13.0 --index-url https://download.pytorch.org/whl/cu130
python -m pip install -r model-runtime/requirements.txt
```

Select **Settings → Model → Speech runtime → Native (experimental)** to try Python-free ASR. Download its separate Q8_0 decoder and audio-projector files (~1.02 GB for 0.6B or ~2.52 GB for 1.7B) and the llama.cpp runtime through Model settings. It reuses the refinement server lifecycle while running a second local server. Native audio is decoded in silence-aligned segments of at most eight seconds; custom dictionary context and token-cap warnings are preserved. The Python sidecar remains available by switching the setting back.

The downloader pins llama.cpp stable **v0.5.0 / b11146**, with checksums for each supported archive. Model downloads use pinned Hugging Face revisions and verify weight SHA-256 before completing. See [runtime-pins.json](docs/runtime-pins.json) for artifact provenance. Native live recognition and its English/Hindi accuracy gate remain unverified, so the default is **Python**; see the [verification report](docs/runtime-fix-report.md).

### Installation

#### Installers

Check [GitHub Releases](https://github.com/Lawliet2004/reflow/releases) for available published installers:

- **Windows**: download the `.msi` or NSIS `.exe` installer.
- **Linux**: download the `.deb` package on Debian/Ubuntu, or the `.AppImage` on other distributions.

macOS installers are published as previews; dictation, shortcuts and startup parity still require physical acceptance. Windows installers are currently unsigned and may show an unverified-publisher warning. macOS previews use ad-hoc signing when Apple signing/notarization secrets are unavailable and can trigger Gatekeeper prompts.

Installers include the compiled application, speech scripts, language metadata and pinned requirements file. Users do not need Node.js, npm or Rust. On first launch, **Download and set up** installs the managed Python speech runtime and selected model through the GUI; internet and several GB of disk space are needed once. Python and model weights are downloaded during setup rather than bundled in the installer. The experimental native runtime is a separate download.

Cloning the source is a development workflow: Node.js, Rust and the platform build prerequisites are needed before the application can open. Run `npm install` and `npm run tauri dev`, then use the same GUI setup; manual `pip install` is optional.

```bash
# 1. Install frontend dependencies
npm install

# 2. Build and verify test suites
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

### First run

The first launch downloads nothing until you choose **Download and set up** in onboarding. That button installs an isolated Python environment, speech dependencies, and the selected model; no system Python, terminal commands, or manual environment activation are needed. If setup is interrupted, use **Retry setup**. Advanced users can select an existing Python environment or the experimental native runtime in Settings. Files are stored in `%APPDATA%\reflow\models` on Windows. With Keep loaded enabled, an installed model loads at startup on the selected available backend. To regenerate the app icon set: `python scripts/generate_icons.py`. To verify the Python ASR pipeline headlessly: `python scripts/test_sidecar.py path/to/speech.wav` (expects an existing recording).

### Linux packages (Debian/Ubuntu)

Build-time:

```bash
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev libgtk-layer-shell-dev libayatana-appindicator3-dev \
  librsvg2-dev libasound2-dev libxdo-dev libssl-dev patchelf pkg-config
```

Runtime: PipeWire or PulseAudio (ALSA via `pipewire-alsa` is enough for `cpal`). The `.deb` also depends on WebKitGTK, GTK 3, Ayatana AppIndicator, and `libxdo3`.

Linux notes:

- Default hotkey is **Ctrl+Shift+Space** (Ctrl+Space is often taken by IBus/fcitx).
- **X11**: global hotkey, overlay, and clipboard paste work. Terminals use **Ctrl+Shift+V**.
- **Wayland shortcuts**: Reflow uses the desktop's GlobalShortcuts portal. Approve the desktop request, then use **Settings → General → Configure desktop shortcuts** to assign the dictation key if your desktop has not assigned it. On KDE Plasma 6.1+ Wayland, modifier-only combinations such as Ctrl+Shift and Shift+Win work as tap-to-start/tap-to-stop shortcuts. Key-bearing shortcuts such as Ctrl+Space, Shift+Space and Ctrl+1 can use hold-to-talk. Other Linux desktops still require a regular key. The Record button remains available while shortcut setup is pending.
- **Wayland paste**: automatic paste requires separate keyboard permission through the desktop's keyboard-only RemoteDesktop portal; Reflow requests no screen or pointer access. Use **Settings → General → Enable automatic paste** to set up access, or **Retry keyboard access** after denial or portal restart. If approval is pending, cancelled, or unavailable, the transcript stays on the clipboard for manual paste. KDE Wayland cannot reliably report the foreground app; choose **Output → Paste shortcut → Ctrl + Shift + V** for terminals.
- **PipeWire inputs**: Audio settings list PipeWire inputs by name. **Automatic** uses a connected headset or Bluetooth mic when one is present, otherwise the built-in mic, and is resolved at the start of each recording. Recording a selected input uses `pw-record` (package `pipewire-bin`); **System default** follows the desktop's default input.
- Data directory: `~/.local/share/reflow`
- Autostart: `~/.config/autostart/reflow.desktop`
- AppImage autostart uses the original `.AppImage` file. Keep that file in a permanent location before enabling Launch at startup.
- Wayland needs `xdg-desktop-portal` and your desktop's portal backend (for example `xdg-desktop-portal-kde` or `xdg-desktop-portal-gnome`). Available shortcut and keyboard-control features depend on that backend. `xdg-utils` provides folder opening.

See [Linux runtime verification](docs/linux-runtime-verification.md) for tested speech paths and desktop-permission checks still awaiting user action.

### Running Reflow Desktop Application

```bash
# Launch Reflow in development mode
npm run tauri dev
```

Build Linux packages with `node scripts/package.cjs`: `.deb` and AppImage. The helper finalizes AppImage launch hooks before hashing; direct Tauri bundles need this finalization step for native Wayland behavior. Runtime: a configured Python interpreter (`REFLOW_PYTHON`, Model settings → Python interpreter, a `reflow/python-runtime` environment, an activated environment, or Python on PATH) plus the pinned speech requirements, or the separately downloaded experimental native runtime and its models. Python and its packages are not bundled inside the AppImage. Pushing a version tag runs CI, creates a draft, builds Windows/Linux installers, macOS previews and the signed Android companion, verifies complete assets and provenance, then automatically publishes the release. There is no separate manual approval in the current workflow. Whether future releases should require manual approval remains a product-policy decision; this audit does not change publication behavior.

### macOS signing (CI)

The release workflow signs and notarizes the macOS installers when an Apple
Developer ID is configured, and falls back to the ad-hoc signature from
`tauri.conf.json` (`signingIdentity: "-"` — also what local builds use) when
the secrets are absent, so forks still produce DMGs. Hardened runtime is
enabled in `bundle.macOS.hardenedRuntime`; the only entitlement is
audio-input.

Repository secrets required for signed builds:

| Secret                       | Contents                                                                                                                                           |
| ---------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `APPLE_CERTIFICATE`          | Base64 of the exported Developer ID `.p12`: `openssl base64 -A -in DeveloperID.p12`                                                                |
| `APPLE_CERTIFICATE_PASSWORD` | Password chosen when exporting the `.p12`                                                                                                          |
| `APPLE_SIGNING_IDENTITY`     | Full identity string, e.g. `Developer ID Application: Name (TEAMID)` — required whenever `APPLE_CERTIFICATE` is set; the job fails fast if missing |
| `APPLE_ID`                   | Apple ID email used for notarization                                                                                                               |
| `APPLE_PASSWORD`             | App-specific password for that Apple ID (appleid.apple.com → Sign-In and Security → App-Specific Passwords)                                        |
| `APPLE_TEAM_ID`              | 10-character team ID from the Apple Developer membership page                                                                                      |

`tauri-action` imports the certificate into a per-run keychain
(`KEYCHAIN_PASSWORD` is generated in the workflow), the bundler signs with
hardened runtime, and `APPLE_ID`/`APPLE_PASSWORD`/`APPLE_TEAM_ID` trigger
notarization. Tauri notarizes the `.app` bundle and staples the ticket onto
it; after bundling, the job verifies the result with
`codesign --verify --deep --strict` plus `xcrun stapler validate` on the
`.app`, and `codesign --verify` on the `.dmg`, failing the release if either
check fails.

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

Speech runtime support floors: managed CPython 3.12 on x86_64 Windows 10/11, glibc Linux x86_64/ARM64 (glibc 2.28+ for torch, 2.27+ for the pinned Zipformer/numpy set), macOS 14+ Apple Silicon or macOS 10.15+ Intel for Zipformer. Qwen/Phonon require AVX2 on x86_64; Zipformer is the lightweight alternative. Windows ARM, 32-bit, musl and specialized Jetson stacks are not verified managed targets. Checked-in target/profile wheel locks and platform checks are applied before large downloads; these floors do not imply microphone, insertion, accuracy or performance acceptance on every platform.
