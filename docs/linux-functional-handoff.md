# New-chat prompt: make Reflow functional on KDE Wayland

Implement this task in `/home/papan/Desktop/Projects/reflow`. This is an implementation and native verification task. Start working; a further plan alone does not satisfy it.

## User intent and completion target

Reflow is an open-source, local desktop dictation alternative to Wispr Flow. The user reports that their Windows workflow worked well, but the migrated Linux application is still **not actually functional**. Treat that report as an unresolved product failure. Find and fix the real failure on this machine before spending substantial time on performance tuning or cosmetic work.

The first milestone is a working release application: launch it normally, focus an isolated text field in another application, activate the configured shortcut, speak, stop or release, receive a transcript, and insert it at the cursor without the HUD stealing focus. Repeat this successfully. Then complete recovery, multilingual recognition, desktop polish, installation, and performance acceptance below.

Preserve Reflow's existing visual identity, custom title bar, supported languages, models, cleanup features, meeting/file transcription, history, privacy, offline operation, and automation contracts. Preserve Windows and X11 compatibility. Research other projects for specific working mechanisms; keep Reflow's architecture and branding.

## Existing work and evidence to read first

The checkout contains extensive uncommitted user work and changes from earlier agents. Begin with `git status --short`, applicable `AGENTS.md` files, and the current diff. Do not reset, clean, stash away, or overwrite this work. Do not replace the application wholesale. Settings additions must be additive and retain saved preferences. Android changes are limited to shared-contract compatibility. Do not commit, tag, publish, or upload a public release unless separately requested.

Read these local reports, then verify their claims against current code and the running application:

- `docs/linux-performance-verification.md`
- `docs/linux-performance-results.json`
- `docs/linux-runtime-verification.md`

Previous verification recorded 239 frontend tests, 835 Rust tests, 36 Python regression tests, formatting/lint/types/Clippy checks, and production builds passing. Separate backend tests exercised 30 microphone cycles, 30 ASR actor cycles, prepared PipeWire speech, and prepared meeting audio. These were **not** 30 complete native shortcut-to-paste dictation cycles.

Native shortcut press/release, real browser/editor/Konsole insertion, permission dialogs, clipboard integrity during insertion, Bluetooth recovery, real suspend/resume, graceful tray Quit, and KDE UI/scaling remained unverified. A Torch CUDA tensor operation passed; actual Qwen CUDA recognition, Hindi accuracy, supported precision comparisons, and cleanup-model coexistence were unverified. The measured speech model was Zipformer 20M CPU INT8 with an English fixture. Do not use it as proof of Hindi support or Qwen execution.

Previous startup/idle measurements were exploratory single-launch observations. ASR median/p95 results did not establish a reproducible speedup. Existing `.deb` and AppImage artifacts passed extraction and CLI smoke checks; this does not establish that their GUI or daily workflow works. The user's latest failure report takes precedence over any broad wording about implementation being complete.

Existing local installers, to inspect and rebuild after repairs:

- `src-tauri/target/release/bundle/deb/Reflow_0.4.0_amd64.deb`
- `src-tauri/target/release/bundle/appimage/Reflow_0.4.0_amd64.AppImage`

## Machine observations from the handoff audit

Observed on 13 October 2026; recheck these rather than treating them as permanent:

| Item                            | Observed state                                                                                   |
| ------------------------------- | ------------------------------------------------------------------------------------------------ |
| OS/session                      | Ubuntu 26.04.1 LTS, KDE Wayland, `wayland-0`                                                     |
| Plasma/KWin                     | 6.6.6                                                                                            |
| Portal/frontend and KDE backend | xdg-desktop-portal 1.21.1; xdg-desktop-portal-kde 6.6.6; both user services active               |
| Portal interfaces               | GlobalShortcuts version 2; RemoteDesktop version 2; AvailableDeviceTypes = 7, including keyboard |
| Audio                           | PipeWire 1.6.2; PipeWire/WirePlumber active; `pw-record` and `pactl` available                   |
| UI libraries                    | libgtk-layer-shell 0.10.0; WebKitGTK 2.52.6                                                      |
| External input/clipboard tools  | `wl-copy`, `wl-paste`, `wtype`, `ydotool`, and `kdotool` absent from the audit shell's PATH      |
| Installed/running Reflow        | No Debian package named `reflow` and no process named exactly `reflow` observed                  |

The earlier report identified Ryzen 5 7535HS, RTX 2050 with 4 GiB VRAM, checkout Python 3.14, Torch 2.13/CUDA 13, and NVIDIA driver 595.99.02. Confirm the current runtime and GPU independently. Missing external tools are not automatically blockers: Reflow already has native clipboard and portal implementations. Interface availability does not prove permission, assignment, signal delivery, or successful input.

## Open-source research and how to use it

The spoken name “open whisper” was interpreted as **OpenWhispr**. Official repositories and protocol documentation were inspected. The following are implementation references, not proof that either project works on this exact machine. Verify current upstream code if it has moved; prefer actual source over stale setup guides or search snippets.

1. **OpenWhispr** was inspected at `fac11281261862efa62c5af4e673d851d92fb4d2`.
   - Its [KDE shortcut manager](https://github.com/OpenWhispr/openwhispr/blob/fac11281261862efa62c5af4e673d851d92fb4d2/src/helpers/kdeShortcut.js) uses KGlobalAccel D-Bus, receives press/release signals, checks conflicts and actual assigned keys, and cleans up registrations. Compare these behaviors with Reflow's existing GlobalShortcuts portal. Consider a KDE-specific fallback only if reproduction establishes that the portal path cannot meet the required behavior.
   - Its [Linux paste implementation](https://github.com/OpenWhispr/openwhispr/blob/fac11281261862efa62c5af4e673d851d92fb4d2/src/helpers/clipboard.js) chooses compositor-specific paths, prefers RemoteDesktop on KDE, distinguishes denial from other failures, serializes paste/restoration, and handles terminal chords. Review it together with the [native helper](https://github.com/OpenWhispr/openwhispr/blob/fac11281261862efa62c5af4e673d851d92fb4d2/resources/linux-fast-paste.c). Successful command dispatch still needs target-field verification. Its optional foreground detection should not be copied as an assumption that generic Wayland exposes active-window information.
   - Its [PipeWire helper manager](https://github.com/OpenWhispr/openwhispr/blob/fac11281261862efa62c5af4e673d851d92fb4d2/src/helpers/linuxPortalAudioManager.js) provides a reference for capability checks, startup/stop timeouts, and unexpected worker exits. Reflow's capture and meeting pipelines must retain their current protocols.
2. **Handy** is especially relevant because it also uses Rust/Tauri. Source was inspected at `14ae9c6b42d729371c443c8617ad02c89c4c8061`.
   - The [Linux notes and troubleshooting](https://github.com/cjpais/Handy/blob/14ae9c6b42d729371c443c8617ad02c89c4c8061/README.md) discuss overlay focus theft, layer-shell startup issues, and configurable WebKit DMA-BUF workarounds. These are diagnostic hypotheses for Reflow on NVIDIA, not permission to disable rendering globally or remove its HUD.
   - Compare its [overlay](https://github.com/cjpais/Handy/blob/14ae9c6b42d729371c443c8617ad02c89c4c8061/src-tauri/src/overlay.rs), [clipboard](https://github.com/cjpais/Handy/blob/14ae9c6b42d729371c443c8617ad02c89c4c8061/src-tauri/src/clipboard.rs), and [owned shortcut worker](https://github.com/cjpais/Handy/blob/14ae9c6b42d729371c443c8617ad02c89c4c8061/src-tauri/src/shortcut/handy_keys.rs). Review lifecycle and failure handling rather than importing the whole stack. At the inspected revision its shortcut abstraction exposed Tauri and HandyKeys; do not assume portal work described in an issue or PR is merged.
3. **XDG Desktop Portal** is the primary contract for Reflow's existing native integration. [GlobalShortcuts](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html) defines session ownership, actual binding results, activation/deactivation, and version-2 configuration. [RemoteDesktop](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.RemoteDesktop.html) defines keyboard grants, cancellation, single-use restore tokens, and input methods. EIS is recommended in the current documentation; legacy Notify methods remain an alternative. Check capabilities and the installed backend before choosing a mechanism.
4. **Rust and compositor libraries:** [ashpd](https://bilelmoussaoui.github.io/ashpd/ashpd/desktop/global_shortcuts/struct.GlobalShortcuts.html) is a typed Rust portal reference if concrete errors in manual zbus handling justify using it. [gtk-layer-shell](https://github.com/wmww/gtk-layer-shell) supports Plasma Wayland; its [Rust initialization contract](https://docs.rs/gtk-layer-shell/latest/gtk_layer_shell/trait.LayerShell.html#method.init_layer_shell) requires initialization before GTK realization. Verify Reflow's actual window lifecycle and layer-surface state, not just a successful function call or an atomic flag.
5. **External tools have limits:** [wtype's source](https://github.com/atx/wtype/blob/master/main.c) requires the virtual-keyboard protocol; it is not a generic solution for every Wayland compositor. [ydotool](https://github.com/ReimuNotMoe/ydotool) uses uinput and requires a persistent daemon/device access. Any optional fallback needs accurate dependency and permission guidance; an unprivileged portal path should not become an unexplained requirement to run the whole application as root.
6. **Audio contract:** [PipeWire's pw-cat/pw-record documentation](https://pipewire.pages.freedesktop.org/pipewire/page_man_pw-cat_1.html) specifies target selection, raw sample formats, resampling, and latency. Use it to check Reflow's exact arguments and device identities.

OpenWhispr and Handy's inspected code licenses were MIT. Retain notices for any reused code, verify the particular dependency's license, and keep Reflow branding. No external repository code was executed during this research. Temporary source snapshots may exist under `/tmp/reflow-linux-handoff.mDuDyl/`; they are optional reading caches, not required dependencies or complete successful checkouts.

## Source map and specific questions to investigate

| Area                                                  | Reflow starting points                                                                                                                                                                    |
| ----------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Startup, single instance, CLI, native window creation | `src-tauri/src/main.rs`, `src-tauri/src/lib.rs`, `src-tauri/tauri.conf.json`, `scripts/kill-running.cjs`                                                                                  |
| Typed desktop state and frontend setup                | `src-tauri/src/platform/desktop.rs`, `src-tauri/src/commands/mod.rs`, `src/hooks/useDesktopIntegration.ts`, `src/components/DesktopIntegrationPanel.tsx`, `src/components/Onboarding.tsx` |
| Shortcut sessions and recording ownership             | `src-tauri/src/hotkey/portal.rs`, `src-tauri/src/hotkey/mod.rs`, `src-tauri/src/session.rs`                                                                                               |
| Keyboard permission, expiry, paste chords, focus      | `src-tauri/src/platform/linux_input_portal.rs`, `src-tauri/src/platform/linux.rs`, `src-tauri/src/platform/keys.rs`, `src-tauri/src/injection/injector.rs`                                |
| Clipboard preservation and fallback                   | `src-tauri/src/injection/clipboard.rs`, `src/components/settings/OutputPage.tsx`, `src-tauri/src/settings/config.rs`                                                                      |
| Mic, selection, meeting capture, preflight            | `src-tauri/src/audio/pipewire.rs`, `capture.rs`, `meeting.rs`, `preflight.rs` in the same directory                                                                                       |
| ASR ownership/runtime/model state                     | `src-tauri/src/asr/actor.rs`, `src-tauri/src/asr/sidecar.rs`, `model-runtime/qwen3_asr_runtime.py`, the model catalog and language catalog                                                |
| HUD, title bar, hidden views                          | `src-tauri/src/overlay.rs`, `src-tauri/src/overlay_layer.rs`, `src/components/OverlayApp.tsx`, `src/components/TitleBar.tsx`, `src/hooks/useViewVisibility.ts`, `src/App.tsx`             |
| Shutdown, suspend, bounded processes, installation    | `src-tauri/src/platform/suspend.rs`, `process.rs`, `linux_desktop.rs`, `sys.rs` in the same directory; `scripts/package.cjs`; Linux metadata; CI workflows                                |

Existing desktop IPC includes `get_desktop_integration_status`, `request_keyboard_access`, `retry_desktop_shortcuts`, and event `desktop:integration-changed`. Preserve these and older IPC/automation contracts. The additive `paste_shortcut` setting supports `auto`, `ctrl_v`, and `ctrl_shift_v`, defaulting to `auto`.

Concrete audit questions, **not confirmed root causes**:

- Is the user opening the expected binary, current frontend assets, and correct data/runtime directory? Is a stale tray instance or deleted-but-running development executable intercepting the new launch? A CLI `--status` in a new process is not proof of the running GUI's state.
- Is the native WebKit UI visible and usable on this NVIDIA/Wayland setup? Compare scoped rendering and layer-shell diagnostic configurations only when a blank window, flicker, or crash is reproduced. Preserve explicit environment overrides. A Vite browser preview cannot establish native integration.
- Does the portal report a saved shortcut while the actual activation/release listeners, binding generations, or dispatcher fail? Are portal dialogs correctly attributed and parented? Does cancellation during setup close the pending request as well as the worker?
- Is the HUD truly initialized before realization, on the correct monitor, and unable to take keyboard focus during dictation? Does a processing or toast transition unexpectedly focus the main window?
- Is transcription successful but delivery blocked by clipboard preservation? Current Wayland code refuses rich HTML, files, privacy hints, unknown formats, or independent mixed image/text offers because arboard cannot fully restore them. That protects data but can prevent normal browser-copy workflows. Make the fallback visible and usable; investigate bounded preservation of supported multiple MIME payloads if needed. Do not silently discard rich data or globally turn off restoration.
- Linux clipboard sequence currently returns `None`. Restoration relies on content comparison, so a new copy with identical plain text or richer data can be mistaken for the old clipboard. Audit races and MIME/ownership changes before claiming preservation.
- Is the chosen microphone still resolvable after reconnect? Is recorded audio non-silent before blaming ASR? Does cleanup wait, runtime discovery, loading state, or cancellation prevent the final transcript from reaching delivery?
- On KDE Wayland, generic foreground information is intentionally unavailable. Use the existing explicit terminal paste shortcut and honest status; do not reintroduce stale XWayland focus data or interactive window pickers into dictation.

## Implementation sequence

### 1. Reproduce and repair a complete dictation

Inspect current launch/configuration/runtime state and obtain a minimal reproduction of the user's failure. If needed, ask one focused question about where it fails while continuing read-only diagnosis. Launch the actual release build with bounded diagnostic logging and a known microphone/model. Avoid logging private transcript/audio content unless necessary and consistent with the user's privacy settings.

Trace one session through native shortcut event → admission/state → microphone start and sample energy → ASR readiness/chunks/final result → optional cleanup → clipboard write/paste dispatch → visible target-field text. Record session identifiers and timing around these transitions. Diagnose with the on-screen Record action, a known speech fixture, and known-text delivery separately, then verify the combined native flow. Fix the first failing boundary and retest it before broad refactoring.

The normal UI must explain pending model loading, missing microphone, unassigned shortcut, pending/denied keyboard permission, transcription failure, and manual delivery. Controls must offer a concrete next step and recover without relaunch where feasible. Saved preferences and installation status must not be presented as live readiness.

### 2. Shortcuts, keyboard permission, insertion, and recovery

Verify both configured toggle and push-to-talk behavior, actual assigned keys, native press/release, repeats, and cancel. Denial, revocation, backend restart, failed rebind, and suspend must not leave recordings or modifiers held. Old generations and cancelled/expired insertion requests must never send keystrokes later. Permission setup is a separate user action, not a queue of delayed transcript insertions.

Use the existing typed status API/events in onboarding, settings, and diagnostics. Gate version-specific portal methods. Restore tokens must follow single-use rotation and revocation semantics. Confirm all setup/retry buttons actually work in KDE. If a supported fallback is necessary, expose its capabilities honestly, including any inability to provide key-release events.

Test native Wayland and relevant XWayland targets in a browser, editor, and Konsole. Confirm Ctrl+V and explicit Ctrl+Shift+V settings reach the actual backend across dictation, history, manual insertion, CLI, and HTTP automation. Keep focus where the user is typing. Successful dispatch is weaker evidence than visible insertion; use accurate status wording and an accessible manual-copy/paste fallback.

Test clipboard restoration with empty/text/Unicode/image/rich-text/file selections, KDE Klipper, a changed selection during processing, and a newly copied selection with the same text. Do not overwrite newer contents. When automatic insertion cannot safely proceed, retain the transcript and make copying/manual pasting practical without hiding an error or erasing the original selection unexpectedly.

### 3. Audio and session lifecycle

Verify physical microphone capture, selected/default devices, stable device identity, Bluetooth disconnect/reconnect, and meeting audio. Device enumeration or selection changes must not interrupt an active stream. Recover at a well-defined next-session boundary; disappearance of an active device must end or fail that session cleanly. Validate rate/channels/resampling/PCM format and sample energy, with zero dropped chunks in controlled tests.

Bound subprocess operations and own/reap workers on cancellation and exit. Run at least 30 complete dictation/cancel/retry cycles, including successful insertion cycles, and check process/thread counts and resource growth. Separately verify graceful tray Quit, relaunch, lock/unlock, portal restart, and real suspend/resume. Do not treat forced parent termination as a substitute for graceful shutdown.

### 4. Recognition and model recovery

Preserve explicit model/device/precision/language choices and every existing supported language. Keep recoverable CPU fallback and show the actual execution backend, fallback reason, loading progress, and errors. Distinguish model installed from model loaded and usable. Verify packaged Python/runtime discovery independently from checkout `.venv` discovery.

Run real Qwen inference on the RTX 2050, not just a CUDA tensor. Use labelled English and Hindi fixtures with a model that supports both. Benchmark only supported precisions within 4 GiB VRAM and include cleanup-model coexistence/OOM recovery. Keep downloads bounded and resumable, observe offline/privacy policy, preserve partial caches, and report unavailable weights honestly. Do not substitute an English-only model or make experimental native ASR the default to obtain a speed result.

### 5. KDE UI, usability, and installation

Keep the design. Fix native title-bar dragging, resize edges, double-click maximize/restore, minimize, close-to-tray, tray restore/quit, focus, HUD placement/interactivity, transparency, and multi-monitor behavior. Check light/dark themes, reduced motion, keyboard navigation and visible focus, and 100%, 125%, and 150% display scaling. Hidden views should stop unnecessary polling/animation and resume with current state; audit visibility/event races instead of assuming the existing hook is correct.

Rebuild and verify `.deb` and AppImage resources, desktop identity, runtime dependencies, menu launch, and autostart. Test launch without an activated shell/checkout environment and after relaunch. A packaged app should provide actionable runtime/dependency guidance. Keep AppImage paths stable after its temporary mount disappears. Check CI coverage and retain Windows/X11 paths; report Windows behavior as unmeasured locally unless genuinely tested elsewhere.

### 6. Performance after functionality

Measure release startup, idle process-tree CPU/RAM, separate model loading, warm recognition, cleanup, insertion, and speech-end-to-visible-text. Compare the same audio, weights, device, precision, settings, and build conditions, with at least 20 warm runs and median/p95. Separate cold loading from warm filesystem/model caches. Recommend a faster configuration only with reproducible latency improvement and no material accuracy regression. Report measured Linux results; do not claim Windows parity from Linux-only observations.

## Verification and delivery requirements

Use available native computer-use tools for interactive desktop checks. The prior session had no native-app control and browser control failed. Recheck current capabilities. If native access remains unavailable, prepare isolated test fields and a short user-assisted test sequence, continue independent repairs, and mark the exact interactive gate pending. Do not fabricate clicks, shortcut tests, clipboard results, screenshots, or acceptance. Follow the current tool environment's UI-control restrictions.

Run checks appropriate to the final changes, including the repository's existing suites:

```bash
npm run lint
npm run format:check
npx tsc --noEmit
npm run test -- --maxWorkers=2
npm run build
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --locked --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --locked --manifest-path src-tauri/Cargo.toml --lib --tests
.venv/bin/python model-runtime/qwen3_asr_runtime.py --selftest
.venv/bin/python -m unittest discover -s model-runtime -p 'test_*.py'
.venv/bin/python -m unittest discover -s scripts -p test_benchmark_runtime.py
.venv/bin/python -m unittest discover -s scripts -p test_zipformer_protocol.py
node --test scripts/package.test.cjs scripts/kill-running.test.cjs scripts/release.test.cjs
```

Inspect CI and package helpers for the current required build steps. Add meaningful regression coverage for reproduced bugs, especially cross-platform contracts, cancellation, permission recovery, clipboard integrity, and runtime discovery. Do not lower accuracy gates, increase test timeouts, or remove features to obtain a passing report.

Deliver the implementation, fresh local release artifacts with hashes, and an evidence-based report. For each acceptance item record **Passed / Failed / Not tested**, the exact build and environment, method, result, and remaining blocker. Separate automated checks, backend hardware tests, native interactive results, and user-assisted observations. Include root causes and before/after behavior, not only test counts.

Acceptance items must explicitly cover: real end-to-end dictation in browser/editor/Konsole; toggle/hold/release/cancel; permission denial/revocation/recovery; clipboard and focus; 30 complete cycles without growing workers; zero dropped chunks in controlled audio; Bluetooth and meeting capture; graceful exit/relaunch/suspend; KDE window/HUD/theme/scaling/accessibility; packaged normal launch/autostart/runtime discovery; English/Hindi and actual Qwen/backend execution; and qualified performance measurements.

Do not call Linux functionality complete while the user's core recording-to-insertion workflow still fails or remains unverified. If a genuine external prerequisite prevents a gate, identify it precisely and finish all work that can proceed independently. Begin now with current-state inspection and reproduction of the actual failure.
