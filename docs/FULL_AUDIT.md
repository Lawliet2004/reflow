# Reflow audit and implementation outcome

Updated 1 October 2026. The audit implementation covers Rust/Tauri, Python/native ASR, LLM runtime/context management, React, Android, history, clipboard insertion, LAN transport, runtime installation, packaging and CI. [The implementation tracker](AUDIT_IMPLEMENTATION.md) maps the full requested scope. [The initial audit](FULL_AUDIT_INITIAL.md) preserves original findings; domain reports are historical assessments rather than the current open-bug list. Existing changes, including [1.7B INT4 repair](int4-asr-fix.md), were retained.

## Automatic runtime and speed

**Auto** chooses supported execution settings from CPU/GPU capability, installed models, live memory headroom, context requirements and observations. **Fast** minimizes wait by skipping LLM refinement. **Balanced/Accurate** retain quality intent; **Custom** preserves explicit settings. Manual placement controls are in Performance, separate from Writing.

To measure settings: open **Settings → Performance → Measure speed on your hardware**, enter the exact short phrase you will read, select its language and record 3–10 seconds. Install speech weights and, if refinement is enabled, writing weights/runtime first. Wait for comparisons, inspect recognized/written text, apply the fastest qualified result and wait for speech to become Ready.

Calibration evaluates installed ASR families/models/supported precisions together with CPU/admitted GPU/half-offloaded writing while both models are resident. One warmup plus three measured repetitions supplies joint processing median/p95, raw/final WER/CER, memory headroom and spill/fallback checks. Inaccurate/unstable candidates cannot win. The search fixes writing context at 1024 and CPU threads at physical-core sizing; it does not exhaustively optimize every thread/context/batch value. Samples remain in memory and are discarded; no insertion, history or model download occurs.

Apply is explicit. Auto reuses qualified results only for the tested forced language and compatible intent/runtime family, hardware/driver, model/runtime file stamps, Python package metadata and embedded inference/prompt/safety code. Live memory admission still applies. Changed inputs invalidate the cache. File stamps are not a fresh full-weight integrity hash. A short phrase qualifies that sample rather than all voices/languages.

The fallback policy selects a small suitable installed ASR for speed and supported CUDA/Vulkan/CPU precision, reserves co-resident model/workspace memory and replans at safe load/launch boundaries. Automatic contexts are bounded to 1024/2048/4096 by hardware capacity, stable across transient free-memory changes; Custom preserves its context. Runtime operations are serialized; recording blocks replacement. Actual loaded configuration and fallback reasons are visible.

### Installed-model joint measurement

On **RTX 2050 (4096 MiB), Ryzen 5 7535HS (6 physical/12 logical cores)**, Python Qwen3-ASR 0.6B CUDA BF16 and Qwen3.5-0.8B processed the same labelled 10.55-second English WAV with both models resident, 1024 writing context, one warmup and three repetitions:

| Writing placement | Joint ASR + writing median | p95         | Worst WER/CER | Observed GPU usage |
| ----------------- | -------------------------- | ----------- | ------------- | ------------------ |
| CPU               | 1307 ms                    | 1328 ms     | 0% / 0%       | 2120 MiB           |
| Full GPU          | **1033 ms**                | **1085 ms** | 0% / 0%       | 2653 MiB           |

GPU writing reduced median processing time by about **21%** among these candidates. [Evidence JSON](implementation-co-resident-calibration.json) and [test log](implementation-co-resident-calibration.log) record the result. It excludes capture/insertion, uses one English sample and deliberately compares two installed configurations. No microphone, download or applied cache was involved. Earlier isolated timings remain in the initial report; they are separate from joint throughput.

## Fixed runtime/context defects

- LLM circuit-breaker deadlock, concurrent launches and dead-child detection.
- Abortable native HTTP and bounded supervised sidecar writes with stalled-reader/hung-request regressions.
- Bounded Python PCM ingress, independent total duration and honest truncation; unsafe parallel partial worker removed; EOS at the cap no longer falsely warns.
- Exact full rendered-template token accounting; sentence/paragraph refinement with one aggregate deadline and complete deterministic fallback; truncated completions rejected.
- Conservative number/signed-value/entity/script/negation/pronoun protection, with original/final recovery. The gate remains lexical rather than semantic equivalence.
- Callback-boundary resampling, default-loader-GPU admission, context-dependent workspace and duplicate resident-memory accounting.
- Preserved supported 1.7B INT4 repair; unsupported 0.6B INT4 remains unavailable. Quantization is not presumed faster.

## Fixed backend/audio/phone defects

- Actual OS default microphone; actionable missing/ambiguous-input feedback, retry, bounded gain/buffer settings and propagated capture faults/drops. Hard-coded microphone-name rejection removed.
- Microphone health and recognition tests; onboarding requires successful recognition for saved settings, and pending/failed saves cannot permit Finish.
- SQLite date/search before pagination, literal substrings, totals/cursors, bounded offsets and local-day clearing; nondestructive corrupt/newer-schema recovery and original/raw/smart/final export.
- Windows rich-format clipboard snapshots with ownership/rapid-paste-chain protection, retry/warning, honest insertion outcomes and original/final recovery/undo edit. Linux/macOS supported clipboard formats remain more limited.
- Generation-scoped WebSocket shutdown on rebinding, bounded blocking handlers, idle revocation and final permission checks after inference before insertion.
- HTTPS/WSS persistent identity, Android certificate pinning, pairing v2/legacy re-pair and independent stream/history/insertion permissions. History/insertion default off; destructive administration stays desktop-only.
- Android lifecycle-owned sessions, stable hold/release, checked AudioRecord/read/stop/join, finite calls, heartbeat/backpressure/send errors and final/error/disposal cleanup; stale callbacks cannot mutate later sessions.
- Immutable runtime staging, archive/manifest/smoke verification, coordinated launch/install, atomic active/previous pointers, inventory/repair/rollback and awaited repair outcomes.

## UI/UX and feature additions

Home has one dictation workflow, stage progress, distinct insertion results, recoverable original/final text and measured speed. Speech/Writing/Performance separate everyday intent from tuning; manual changes select Custom. Serialized saves expose pending/errors; independent status failures do not hide healthy systems.

Added microphone/recognition readiness, labelled calibration, runtime repair/rollback, history export/recovery/comparison, per-application style/dictionary profiles, real ASR segment progress and secure phone permissions. Diagnostics show actual RTF, backlog/drops, warm state, p50/p95 and LLM prompt/decode timing/token rate where supplied. Unmeasured values remain unmeasured. Shared controls have labels/errors, visible focus, readable narrow layouts and actionable recovery.

### Audio languages

One catalogue supplies **30 forced choices plus Auto-detect** across desktop/Rust/Python/Android: Arabic, Cantonese, Chinese, Czech, Danish, Dutch, English, Filipino, Finnish, French, German, Greek, Hindi, Hungarian, Indonesian, Italian, Japanese, Korean, Macedonian, Malay, Persian, Polish, Portuguese, Romanian, Russian, Spanish, Swedish, Thai, Turkish and Vietnamese. Writing preserves language/script/code mixing.

Qwen3-ASR's 22 Chinese dialects belong to automatic recognition, not extra forced-language values. Languages absent from ASR's supported set are not advertised solely because the LLM can write them. Sources: [ASR model card](https://huggingface.co/Qwen/Qwen3-ASR-0.6B), [ASR language utility](https://github.com/QwenLM/Qwen3-ASR/blob/main/qwen_asr/inference/utils.py), [Qwen3.5 model card](https://huggingface.co/Qwen/Qwen3.5-0.8B). Declared coverage is separate from representative accuracy across all languages.

## Removed or simplified

Removed inert mDNS settings, CAMERA permission, unused VAD pre-roll, unsafe disabled partial inference, ineffective substring index and base64 SVG wrapper around the original logo PNG. Migrations, user data and original visual identity remain. Manual tuning is consolidated behind Custom, static speed claims use real status/observations and packaging no longer claims pretend signing/success. Tested compatibility/diagnostic components remain; image-container changes do not establish a paint-speed improvement.

## Verification and dependencies

Fresh checks passed:

| Check                          | Result                                                                                                                                 | Evidence                                                                                                                                                              |
| ------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Frontend                       | 109 tests in 28 files; lint with zero warnings/errors; formatting and TypeScript pass                                                  | [Complete check](implementation-web-release-check.log)                                                                                                                |
| Production frontend build      | TypeScript/Vite pass                                                                                                                   | [Build](implementation-build-release-check.log)                                                                                                                       |
| Rust library/integration suite | 463 passed; opt-in calibration ignored by default                                                                                      | [Fresh full suite](implementation-rust-release-check.log)                                                                                                             |
| Installed joint calibration    | Separate opt-in test passed with qualified winner                                                                                      | [Live calibration](implementation-co-resident-calibration.log)                                                                                                        |
| Rust lint/format               | Strict all-target clippy and cargo fmt --check pass                                                                                    | [Clippy](implementation-clippy-release-check.log)                                                                                                                     |
| Python                         | 14 runtime regressions, 9 quality-metric tests, 70 selftest checks pass                                                                | [Runtime](implementation-python-release-check.log), [quality](implementation-quality-release-check.log), [selftest](implementation-python-selftest-release-check.log) |
| Android                        | 27 JVM tests, debug APK and lint pass                                                                                                  | [Build/lint](implementation-android-final.log)                                                                                                                        |
| Packaging                      | 11 contract/process-ownership tests pass; wrapper dry-runs and workflow validation pass                                                | [Tests](implementation-packaging-release-check.log), [workflow](implementation-ci-validation.log)                                                                     |
| Browser/UI                     | Speech and Performance inspected at 640 px; no horizontal overflow or captured console errors; source UI detector returned no findings | [QA scope](implementation-browser-qa.md), [detector](implementation-ui-detector.json)                                                                                 |

These counts describe individual suites rather than adding overlapping reruns. Source files were also scanned for NUL corruption and git diff --check passed. The opt-in installed calibration test does not run automatically in CI because it requires local installed weights and a labelled WAV fixture.

[npm audit](audit-dependencies-final.json) reports zero vulnerabilities. [Cargo audit](implementation-rust-dependencies.json), using the fetched DB without a fresh fetch or yanked-registry check, reports zero vulnerability entries, six unmaintained warnings and one GLib unsound advisory. Enigo 0.3 brings memmap2 0.9.11, fixing [RUSTSEC-2026-0186](https://rustsec.org/advisories/RUSTSEC-2026-0186.html). Tauri's Linux GTK chain requires GLib 0.18.5; [RUSTSEC-2024-0429](https://rustsec.org/advisories/RUSTSEC-2024-0429.html) is fixed from 0.20. Adding a separate newer GLib does not fix old consumers. Reflow does not directly call VariantStrIter, which does not establish safety for every upstream path.

## Remaining physical/platform work

- Physical Android recording/rotation/interruption, microphone unplug, rich-clipboard restoration, native keyboard/screen-reader and target-app insertion require OS/device QA; regression implementation is complete.
- Native ASR weights are absent here. Cancellation/transport is tested; native decoding quality and Python/native comparison need installed weights and labelled audio.
- CPAL lacks stable endpoint IDs; ambiguous explicit names are rejected. Use System default and the OS selector in that case.
- GPU 0 is the admission target. Arbitrary multi-GPU/remapped CUDA/mixed-vendor Vulkan identity, thermal/battery tuning and continuous live allocation migration need separate support. Auto changes execution at safe load boundaries.
- Windows is the primary verified host. Linux compositor injection and upstream GLib need platform validation. macOS native dictation/hotkey/startup parity remains incomplete, so public macOS installers stay gated despite configured compile/tests.
- No installer was built/installed/signed or release published. Credentials and real installation remain release requirements. [Packaging details](packaging-validation.md) record actual commands/checksums/gates. Hosted CI has not run for this checkout.
- Calibration optimizes a bounded measured set, not a global optimum or representative multilingual corpus.

Changes remain reviewable in the working tree. No commit, publishing action or user-data deletion was requested.
