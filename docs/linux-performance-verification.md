# Linux performance and polish verification

Verified on 12–13 October 2026: Ubuntu 26.04.1 LTS, KDE Wayland, Ryzen 5 7535HS (6 cores / 12 threads), RTX 2050 (4 GiB), Python 3.14, Torch 2.13.0 / CUDA 13.0, NVIDIA driver 595.99.02. Application version remains 0.4.0.

These are historical checks of the earlier implementation. Subsequent release-launch failures are documented in [Linux functional repair verification](linux-functional-verification.md), which takes precedence. Native interactive acceptance and Qwen/Hindi performance qualification remain incomplete. No faster model or precision is recommended from these results.

## Changes and audit findings

The starting checkout contained substantial uncommitted Linux work. It was preserved. Its first Rust build failed because the HUD used GTK layer-shell functions without enabling their Cargo feature. The baseline release includes only that prerequisite repair; it is not an untouched HEAD build.

- **Desktop integration:** Added `get_desktop_integration_status` and `desktop:integration-changed`, with assigned shortcuts, keyboard consent, HUD support, and foreground-information availability. Existing platform IPC and hotkey events remain available. Setup and General settings share the status and offer permission setup, retry, desktop configuration, and reconnection. Ready events clear obsolete shortcut warnings.
- **Session ownership:** Shortcut-session closure cancels only recordings owned by its bindings. Wayland also registers the configured cancel action and reports its actual desktop assignment. Portal owner changes invalidate the session. Keyboard approval does not enqueue input; expired or cancelled requests cannot send later keystrokes, including across elapsed suspend time.
- **Audio:** Desktop probes are bounded and reaped without detached readers. Device enumeration and microphone-test startup run outside the UI thread. The idle microphone probe no longer holds recording admission. Selected capture streams own their workers; microphone selection is resolved at recording start. Existing meeting mixing, audio retention, and model protocols remain intact.
- **Recognition and delivery:** Final insertion and manual IPC insertion run outside asynchronous/UI threads. The additive `paste_shortcut` setting defaults to `auto`; explicit Ctrl+V and Ctrl+Shift+V also apply to CLI, history repaste, and existing automation endpoints. Qwen defaults, precision preferences, multilingual catalog, and the experimental native-runtime gate were retained.
- **Clipboard:** Automatic failure leaves the transcript available for manual paste and retains a restoration snapshot when restoration is enabled. Settings → Privacy & diagnostics → Retry clipboard restore can restore it after manual paste. Retries refuse changed clipboard content; Windows also checks the clipboard sequence. Wayland snapshots refuse rich text, files, privacy hints, or independent mixed formats that the text/image backend cannot fully restore; the original clipboard is left unchanged and the transcript remains in Reflow.
- **Windows and HUD:** Removed the title bar's duplicate maximize handler and use Tauri's drag-region handling. Tray restoration unminimizes the main window. GTK layer-shell initializes after portable windows exist. Hidden main/HUD windows pause animation; all recurring view polls stop while hidden. File progress uses backend events with a polling fallback and protects newer events from stale responses. Decorative preview animation respects system reduced motion.
- **Shutdown and suspend:** Linux watches login-manager sleep notifications, cancels capture, closes keyboard/shortcut sessions, and reconnects on resume. Exit stops owned capture/refinement workers and requests bounded ASR shutdown. Linux runtime/capture children also receive a parent-death signal.
- **Storage and downloads:** Existing session/settings serialization, history recovery and encryption safeguards, pinned downloads, network-policy enforcement, and bounded retries were retained. Failed autostart writes roll back the saved startup flag and report the error.
- **Installation:** Added GTK layer-shell development dependencies to Linux CI/release jobs. Packaging tools run in extraction mode on machines lacking FUSE 2. Benchmark discovery now follows Linux XDG, Windows APPDATA, macOS, explicit data-directory overrides, and the current refinement-runtime inventory. AppStream metadata references the actual `Reflow.desktop` launcher.

Android source was not changed. Windows-only behavior remains covered by the configured Windows CI job; that job was not executed on this machine.

## Automated checks

| Check                                                    | Result                                                       |
| -------------------------------------------------------- | ------------------------------------------------------------ |
| Frontend ESLint, Prettier, TypeScript, production build  | Passed                                                       |
| Frontend tests                                           | 239 passed in 45 files                                       |
| Rust formatting and Clippy, all targets, warnings denied | Passed                                                       |
| Rust unit/integration tests                              | 835 passed, 0 failed, 15 environment-dependent tests ignored |
| Python runtime selftest and model-free regressions       | Passed; 36 regression tests                                  |
| Benchmark/path regressions                               | 11 passed                                                    |
| Zipformer protocol regressions                           | 3 passed                                                     |
| Node packaging/platform/release regressions              | 17 passed, 2 platform-specific skips                         |
| AppStream validation, without network                    | Passed                                                       |
| Working-tree whitespace validation                       | Passed                                                       |

A loaded parallel frontend run hit timeouts; the complete fresh run with `npm run test -- --maxWorkers=2` passed without increasing timeouts.

Hardware checks were run separately from the default suite:

- 30 one-second microphone capture/cancel cycles: zero dropped chunks, finite audio, no growing capture process/thread counts.
- 30 Rust ASR actor record/transcribe/cancel/recovery cycles with the same installed Zipformer: stable transcript and no growing actor process/thread counts.
- Isolated PipeWire monitor capture → resampling → Rust actor → English recognition: passed; zero dropped chunks, peak 0.535, approximately 9.92 seconds captured.
- Meeting capture from a prepared virtual microphone plus playback monitor: passed, finite non-silent mixed audio and zero dropped chunks. Temporary PipeWire modules were removed afterward.
- A CUDA tensor operation executed successfully on the RTX 2050. This verifies Torch CUDA execution, not Qwen execution or accuracy.

## Measurements and their limits

Raw observations are in [linux-performance-results.json](linux-performance-results.json). `scripts/measure_linux_desktop.py` measures an isolated portable release, hidden in the tray, with the same Zipformer settings. API serving, autostart, and history/audio retention are disabled only in the isolated test configuration. CPU is a percentage of one logical core; memory sums owned-process RSS and can count shared pages more than once.

| Release observation                                  | Prerequisite-repaired baseline | Updated release |
| ---------------------------------------------------- | -----------------------------: | --------------: |
| Launch → model-ready log                             |                       5,936 ms |        4,142 ms |
| Idle CPU, mean over 30 one-second samples            |                          0.95% |           0.69% |
| Idle CPU, median                                     |                             0% |              0% |
| Idle process-tree RSS, median                        |                   1,621.10 MiB |    1,621.06 MiB |
| Children still alive 500 ms after forced termination |                              1 |               0 |

These are exploratory single-launch observations, not proof of a reproducible startup/CPU speedup or a memory reduction. Filesystem caches and background scheduling were not reset. The survivor count describes the short observation interval; it does not establish a permanent baseline leak. The harness cleans up the captured process identities. Graceful tray Quit remains a native acceptance item.

The offline speech comparison used a 6.625-second, 16 kHz mono PCM16 English fixture from the [official Zipformer example](https://k2-fsa.github.io/sherpa/onnx/pretrained_models/online-transducer/zipformer-transducer-models.html). Its SHA-256 is `6bc58a4efdf20daac252b6b1502632601a71efe0308f6757dc1eda34891a7e4f`. The labelled reference was checked against [the source passage](https://www.gutenberg.org/cache/epub/4217/pg4217-images.html). Both versions use identical weights, CPU INT8, one untimed warmup, then 30 sequential warm repetitions.

| Speech observation                         |   Baseline runtime |    Updated runtime |
| ------------------------------------------ | -----------------: | -----------------: |
| Load in a new process, measured separately |             5.05 s |             3.38 s |
| Complete offline recognition, median / p95 | 166.67 / 179.13 ms | 164.88 / 189.92 ms |
| Final-stream drain, median / p95           |     7.51 / 8.24 ms |     7.27 / 8.74 ms |
| WER / CER                                  |      5.56% / 3.45% |      5.56% / 3.45% |

Loading in a new process uses warm filesystem caches and includes Python native imports; it is not a cold-boot benchmark. Recognition timing includes protocol ingestion and incremental decoding, without real-time playback duration. Final drain is only the remaining work at stop. GUI model readiness and Rust actor/capture were checked separately. No insertion or speech-end-to-inserted latency is claimed. The speech implementation did not change, accuracy on this fixture is unchanged, and the tail timing does not support an ASR speedup claim.

The Python benchmark now defaults to at least 20 warm runs, reports per-language median/p95 and WER/CER, and keeps model loading and warmup separate. It uses installed models without downloads or settings changes:

```bash
.venv/bin/python scripts/benchmark_runtime.py corpus.json output.json \
  --model zipformer-20m --device cpu --precisions int8 --repeats 30 --skip-llm
```

The corpus is an array of `{ "language": "en", "audio": "speech.wav", "reference": "labelled speech" }` records. Qwen qualification should supply both English and Hindi records, installed Qwen weights, and supported precision choices. The measured Zipformer error exceeds the existing strict calibration threshold, so this fixture does not justify a new configuration recommendation.

## Native acceptance still required

The desktop portal reported the existing Ctrl+Shift+Space assignment during release launches. Receiving native press/release events and actual insertion were not verified. This session exposes no native computer-use windows; browser control also failed with `Unable to load browser request-header policy` after retry. Consequently, these remain unverified:

- Shortcut press/release, global cancel, permission denial/revocation and live portal restart against real desktop applications.
- Automatic paste and clipboard restoration in a browser, editor, and Konsole; insertion timing.
- Bluetooth disconnect/reconnect, physical-device switching during a recording, graceful tray Quit, and real suspend/resume.
- KDE dragging/resizing/maximize, tray and HUD behavior, light/dark appearance, keyboard navigation, transparency, and 100%, 125%, 150% display scaling.
- Qwen CUDA model loading, BF16/INT8/INT4 latency and VRAM, Hindi accuracy, and coexistence with cleanup models. Qwen weights and cleanup models are not installed; Phonon remains partially downloaded.
- Comparable Windows performance and behavior on other Linux distributions. No parity claim is made.

The existing language catalog tests passed. They do not substitute for multilingual speech accuracy measurements. Current saved model choices were preserved; experimental native ASR was not made the default.

## Release artifacts

Local installers are generated under `src-tauri/target/release/bundle/`, with sibling SHA-256 files:

- `deb/Reflow_0.4.0_amd64.deb`
- `appimage/Reflow_0.4.0_amd64.AppImage`

The packages are built on the Ubuntu 26.04 acceptance machine. Package extraction, desktop metadata, runtime resource contents, dynamic dependencies, and packaged `--status` smoke checks are checked locally. These checks do not install packages system-wide or certify older distributions. Without FUSE 2, use the AppImage runtime's extraction mode. The configured Ubuntu 22.04 release job supplies a separate build target for distribution.

No commit, tag, upload, or public release was made. All original uncommitted work remains in the checkout.
