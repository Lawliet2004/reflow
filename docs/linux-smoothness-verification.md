# Linux smoothness refactor — 14 October 2026

This iteration targets the reported choppy scrolling/page changes and shortcut settings that did not match KDE's active binding. It retains the existing design, speech pipeline, saved choices, installed weights and unrelated dirty checkout changes. The source baseline is `/tmp/reflow-smoothness-baseline-20261013`, rather than Git HEAD. Prior microphone/HUD repairs and the user's successful live dictation test are documented in [microphone-hud-fix-2026-10-13.md](microphone-hud-fix-2026-10-13.md).

## Changes

- The native hub retains microphone levels/partial transcripts in a ref while History, Notes or Settings is open. These unused updates no longer reconcile the entire window. Returning Home restores current text; a new recording clears the previous session. The separate native HUD keeps its own event stream. Browser preview retains its shared overlay updates.
- Intelligence polling stops on pages without consumers, resumes immediately on return and stays event-driven for downloads and terminal refreshes. Model/speech readiness events and the navigation recording indicator remain active.
- Waveform bars retain fixed geometry and animate visual scale. Reduced-motion and hidden-view behavior remain supported.
- Hardware probes, metrics, runtime plans, diagnostics, model directory scans, GPU-backed refinement status and model reloads run outside GTK's event loop. Concurrent cached capability misses share a serialized probe; the snapshot mutex is never held while subprocesses run. Linux reuses the existing bounded process runner, including output draining, timeout, output cap, process-group cleanup and AppImage host environment handling.
- An initial observation of AC power no longer unloads/reloads an already loaded speech model. Only a known battery-to-AC transition triggers the existing resume policy. Missing power readings preserve the last known transition state.
- On Wayland, shortcut controls display the desktop's actual assignment and open its real configuration. Home and onboarding follow that assignment. Saving a preferred key no longer falsely presents it as an active global key. Optional actions can still be enabled/cleared. Registration/permission errors remain visible and retryable.
- Cancel participates in the same syntax/conflict validation as other actions before settings are saved or bindings removed. Custom cancel combinations are supported; Windows keeps its transparent bare-Escape hook. Invalid/empty/conflicting keys preserve saved settings, including conflicts hidden by modifier order/aliases. Native cancel follows the registered binding; the UI's Cancel button remains available.
- Shortcut hints no longer hardcode Escape. The sidebar uses the reported speech model name, including Phonon-2. Refinement's VRAM field reports sampled device-wide **used** memory rather than total capacity; it is not a per-model allocation measurement.

## Delegated review

Devin CLI was invoked with the verified model ID `swe-2-max`. Three focused workers reviewed frontend performance, shortcut configuration, and hardware/model behavior concurrently. A subsequent focused review checked the integrated performance changes. A further cancellation review produced no response after over seven minutes and was stopped; cancellation changes were reviewed by the primary agent and validated with registry, persistence and desktop-event regressions. The maximum concurrent worker count was three. Source packets instructed workers not to call tools; proposals were reviewed, adapted, applied and tested by the primary agent.

Adaptations deliberately avoid duplicate desktop-status subscriptions/buttons, presenting requested keys as assigned keys, an unbounded subprocess-reader thread, and a proposed battery-state type error. Probe coalescing uses an actual serialized/rechecked miss rather than timestamp ordering alone. The final performance review's declaration-order concern was checked against the complete App source: both state declarations and the `isTauri` import precede their use. Its consumer/polling questions were checked against all three navigation pages and Settings. All callers of the newly fallible commands handle rejection. The existing process runner checks unsuccessful exits and kills/reaps owned subprocesses. Transcript payloads are replaced rather than mutating the shared reset constant.

## Evidence and limits

Raw render measurements and the changed-file inventory are in [linux-smoothness-results.json](linux-smoothness-results.json).

| Identical frontend workload                                            |    Before |   After |
| ---------------------------------------------------------------------- | --------: | ------: |
| Whole-window commits from 30 microphone-level events with History open |        30 |       0 |
| Synchronous workload time in the recorded run                          | 114.31 ms | 0.75 ms |

This is React Profiler in jsdom with the native branch mocked. It proves unnecessary reconciliation was removed for that workload; it does **not** establish native FPS, scroll latency, battery savings or perceived smoothness. Timing varies with machine load. Returning Home and retaining the transcript, including clearing a new session, are regression assertions.

Validation completed after the final source edits:

| Check                                    | Result                                                    |
| ---------------------------------------- | --------------------------------------------------------- |
| ESLint, Prettier and TypeScript          | Passed                                                    |
| Frontend Vitest                          | 248 passed in 47 files                                    |
| Rust all-target unit/integration suite   | 865 passed; 18 opt-in/environment-dependent tests ignored |
| Clippy with warnings denied; rustfmt     | Passed                                                    |
| Python runtime selftest                  | 70 successful checks                                      |
| Python model-free regression suite       | 39 passed                                                 |
| Node packaging/process/release contracts | 19 passed; 2 platform-specific skips                      |
| Installed Phonon-2 actor smoke test      | 30 recognition/cancel/recovery cycles passed              |
| Working-tree whitespace check            | Passed                                                    |

The final optimized Tauri build passed (`npm run tauri -- build --no-bundle`), including all final source changes. The binary is `src-tauri/target/release/reflow`. The frontend bundle emits Vite's existing >500 kB chunk advisory (about 509 kB raw / 151 kB gzip); there is no build failure. Native acceptance remains pending. Installed Phonon-2 passed 30 real Rust actor/Python recognition-cancel-recovery cycles against the existing English WAV fixture, with no growing worker/thread counts. This checks runtime recovery, not microphone input, Hindi accuracy, global key delivery or visible insertion.

The saved configuration remains Phonon-2 with automatic device/precision, automatic connected microphone selection, and speech-only cleanup (`flow_model: none`). No LLM model was downloaded or selected. Existing model-free hardware/profile/fallback/cleanup tests exercise those contracts; live GPU LLM inference remains unqualified without installed weights. Windows, X11 and other Linux hardware are not claimed as live-tested.

The laptop internal microphone still requires the existing Secure Boot driver-key repair; external earbuds were previously verified. Universal hardware compatibility cannot be inferred from this single machine. Native scrolling, navigation and actual desktop rebinding require the user's test of the updated optimized app. Diagnostic logs remain until that explicit confirmation.

## Public references

The blocking-command decision follows [Tauri's Rust command threading documentation](https://v2.tauri.app/develop/calling-rust/). Desktop assignment follows the [GlobalShortcuts portal contract](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html). Public [Wispr Flow Linux guidance](https://docs.wisprflow.ai/articles/6478598909-using-flow-with-linux-wsl-and-terminal-applications) and [OpenWhispr's source/troubleshooting](https://github.com/OpenWhispr/openwhispr/blob/main/TROUBLESHOOTING.md) were reviewed as behavioral references. No proprietary internals were obtained or claimed. The supplied [REA repository](https://github.com/morluto/rea) was inspected; its decompilation setup was unnecessary for this available-source repair and was not installed.

## Follow-up: in-app KDE shortcut editor

The subsequent user-requested in-app picker is implemented and validated in [in-app-shortcuts-linux.md](in-app-shortcuts-linux.md). On KDE, the General and onboarding fields now capture a key and apply the native assignment directly. Other desktops retain their configuration fallback. That follow-up report records updated checks, native observations and the corrected test incident.
