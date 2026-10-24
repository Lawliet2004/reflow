# Reflow application audit — 24 October 2026

**Assessment: the current tree is not ready to promise that users will never need to troubleshoot setup or runtime failures.** The automated functional suites are strong and pass, but there are outstanding privacy, retention, model-lifecycle, resource-admission, context-management and release-gate issues.

This audit records **7 findings: 4 P1 and 3 P2**. P1 means fix before broad distribution; P2 means a material reliability or experience problem. No P0 was established. Validation gaps below are separate from confirmed bugs.

## Scope and evidence

Audited HEAD: `cc31970a02d1c86da9a3e881fcd9fd5e0c13b2b9`, **including existing uncommitted changes**. This is a review of the current working tree, not certification of an already published installer. The checkout already contained substantial model/catalog and runtime work. Application code, dependencies, user settings and model installations were not changed by this audit. A temporary Rust example was created for a disposable-data probe and removed; its source and results are retained here.

Reviewed paths include onboarding and setup ownership; managed Python profiles; speech download/load/reload; native and Python runtime selection; cleanup, command and meeting context handling; memory admission; recording/session lifecycle; meeting persistence and encryption; history retention and recovery; LAN authentication and permission boundaries; packaging and CI; and the React settings/model surfaces. Existing audit documents were treated as historical evidence, not current results.

Evidence labels:

- **Reproduced:** executed in this audit using the current code or toolchain.
- **Code-confirmed:** a concrete problematic path exists; the full hardware/user scenario was not executed.
- **Validation gap:** insufficient acceptance evidence; not a claim that the feature is broken.

No finite audit can prove the absence of every bug. In particular, this run did not perform fresh multi-GB model installations, physical microphone/insertion tests, Windows/macOS execution, Android device tests, or a five-hour physical meeting soak.

## Verification results

| Check                              | Result                                                                                                                  | Evidence                                                                           |
| ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| Full Rust tests                    | **1,034 passed, 30 ignored**, 47 result blocks including doc-tests                                                      | [rust.log](evidence/rust.log)                                                      |
| Frontend tests, one worker         | **407 passed in 60 files**                                                                                              | [frontend-serial.log](evidence/frontend-serial.log)                                |
| Initial parallel frontend run      | 400 passed, 7 failed under concurrent Rust compilation; all pass on bounded rerun                                       | [frontend.log](evidence/frontend.log)                                              |
| Production TypeScript/Vite build   | Passed; main JS chunk **605.74 kB**, 177.18 kB gzip; size warning                                                       | [build.log](evidence/build.log)                                                    |
| ESLint                             | No errors; one existing state-in-effect warning                                                                         | [lint.log](evidence/lint.log)                                                      |
| Prettier gate                      | **Failed: five files**                                                                                                  | [format.log](evidence/format.log)                                                  |
| Rust formatting                    | Passed                                                                                                                  | [rust-format.log](evidence/rust-format.log)                                        |
| All-target Clippy, warnings denied | **Failed: five diagnostics**                                                                                            | [clippy.log](evidence/clippy.log)                                                  |
| Python model-free regressions      | **50 passed** using existing managed Python                                                                             | [python-managed.log](evidence/python-managed.log)                                  |
| System Python attempt              | Missing NumPy; environment limitation, not an ASR product failure                                                       | [python.log](evidence/python.log)                                                  |
| Packaging/release Node tests       | **20 passed, 2 platform skips**                                                                                         | [packaging.log](evidence/packaging.log)                                            |
| npm advisory scan                  | **Two high-severity dependency entries**, both development dependencies                                                 | [npm-audit.json](evidence/npm-audit.json)                                          |
| Storage/context probe              | Encryption success with plaintext WAL; context slices exceed advertised shared reservation                              | [probe.log](evidence/probe.log), [source](evidence/probe.rs)                       |
| UI detector                        | Completed with no output; not an accessibility certification                                                            | [ui-detector.log](evidence/ui-detector.log)                                        |
| Browser preview                    | Home/settings/model catalog inspected, including viewport overrides at 640×480 and 1040×780; preview uses fallback data | Native permission, insertion and model readiness remain unverified by this preview |

The frontend rerun establishes that the seven initial failures are not persistent failures under the bounded configuration. It does not establish that the default worker configuration is reliable on every low-memory development machine. The functional test successes also do not override the failing CI gates.

## Findings

### F01 — P1: meeting encryption can report success while plaintext remains in the WAL

**Reproduced.** [Meeting store encryption](../../src-tauri/src/meetings/store.rs:262) commits encrypted replacements, then executes `PRAGMA wal_checkpoint(TRUNCATE); VACUUM;` through `execute_batch`. It does not inspect the checkpoint's busy result. Another open SQLite read transaction can retain pre-encryption WAL frames.

The isolated probe inserted a synthetic private transcript, opened a second reader transaction, and enabled encryption with an injected test key. The method returned `Ok(())`, the meeting row reported `encrypted=true`, and the WAL still contained `AUDIT_PRIVATE_PLAINTEXT_MARKER`. No user database or OS key store was used. The exposure demonstrated is during the held reader; this is not a claim that plaintext survives every subsequent clean shutdown/checkpoint.

**Impact:** the application can claim encryption succeeded while recoverable plaintext transcript bytes remain on disk. The ordinary history database already takes stronger precautions around journal transitions; the meeting store does not provide equivalent assurance.

**Fix:** establish an exclusive, checked journal transition before converting data; fail safely if readers prevent it. Verify checkpoint results and old WAL removal before reporting success. Add a regression with a second connection holding a read snapshot and inspect both the main database and WAL.

### F02 — P1: meeting audio and transcript retention do not expire automatically

**Code-confirmed.** [The periodic retention worker](../../src-tauri/src/lib.rs:134) cleans only `history_store`. [Meeting retention](../../src-tauri/src/meetings/store.rs:322) is invoked by [settings changes](../../src-tauri/src/commands/mod.rs:703), but has no production periodic/startup call. Processing separately removes meeting audio when retention is already disabled; that does not implement time-based expiry.

**Trigger:** choose one-day audio retention, finish processing a meeting, then leave the policy unchanged. Once the recording becomes older than one day, the normal 30-second cleanup worker never visits it. Restart recovery does not add the missing expiry call. The same gap applies to time-limited meeting transcript retention.

**Impact:** meeting data can remain beyond the deletion period promised by Advanced settings, consuming disk and extending sensitive-data retention. The deliberate protection of unfinished/failed jobs is separate and should remain explicit.

**Fix:** include the meeting store in startup and periodic cleanup under appropriate capture/processing coordination. Test an expired **ready** meeting while leaving settings unchanged, including encrypted storage. Preserve the documented recoverable-input exception for unfinished work.

### F03 — P1: summary model launches bypass host-RAM admission

**Code-confirmed; no intentional OOM performed.** Automatic dictation planning checks available RAM in [runtime planning](../../src-tauri/src/runtime.rs:144), but [history summarization](../../src-tauri/src/expansion_commands.rs:23) and [meeting summarization](../../src-tauri/src/meetings/process.rs:311) call `FlowRuntime::ensure` directly using saved settings. The final launch loop's memory check is conditional on a **native ASR** manifest, not a writing model: [server launch admission](../../src-tauri/src/rewrite/server.rs:692).

**Trigger:** select a large installed summary model on a memory-constrained/busy computer and summarize history or a meeting. The history route also leaves the speech model resident. Meeting processing unloads ASR, which helps, but still does not establish that the requested LLM and context fit available RAM. GPU launch failure can enter the same unchecked CPU fallback.

**Impact:** a model can be downloaded successfully yet summary execution can push the machine into heavy paging, fail after a long load, or be terminated for memory pressure. The new ~4.98 GB Gemma weights make this particularly important. These outcomes are risks implied by the missing admission check, not reproduced OOM events.

**Fix:** enforce writing-model RAM/context admission at the shared final launch boundary, including CPU fallback, and communicate a smaller-model/retry option. Do not rely solely on the dictation preview planner. Test low-RAM summary launches and GPU-to-CPU fallback without allocating the real oversized model.

### F04 — P1: the current release-validation gates are red

**Reproduced.** Functional tests and build pass, but `.github/workflows/ci.yml` requires additional checks that fail in this tree:

- Clippy: `audio/capture.rs:710` too many arguments; `meetings/inhibit.rs:10` needless return; `meetings/capture.rs:614` items after test module; `settings/config.rs:1617` and `:1634` field reassignment after default construction.
- Prettier: the model-page surface brief, `docs/model-runtime-additions.md`, and three meeting verification JSON files listed in the captured log.
- npm security: `undici@7.30.0` via `jsdom`, and `zod@4.4.3` via `eslint-plugin-react-hooks`. Both entries are high severity. Upstream patched versions are [Undici 7.30.1](https://github.com/advisories/GHSA-xjj6-f33q-929c) and [Zod 4.6.6](https://github.com/advisories/GHSA-mrhx-c7xc-m3w5); the advisories were checked during this audit.

**Impact:** the required validation cannot go green on the audited tree. The advisory findings concern the development/test toolchain; this audit does **not** establish a corresponding exploit in the packaged React/Tauri application.

**Fix:** correct the lint diagnostics, format the listed files, refresh the affected compatible dependency locks, and rerun the exact CI commands. Do not weaken the gates to make this result green.

### F05 — P2: switching an installed speech model can skip its dependency setup

**Code-confirmed.** [Model selection](../../src/components/settings/ModelPage.tsx:315) invokes `setupSpeechModel` only when model **weights** are not installed. [The managed Python interpreter](../../src-tauri/src/asr/python_setup.rs:375) is selected by one active-generation pointer, while readiness is explicitly [profile/device-specific](../../src-tauri/src/asr/python_setup.rs:521). The deferred reload path loads weights without performing the missing profile setup.

**Trigger:** install Qwen, then install a lightweight Zipformer environment, keeping both sets of weights. Selecting the already-installed Qwen row skips dependency preparation and tries the current interpreter, which may lack Qwen/Torch. Older verified generations are retained but are not automatically selected by requested profile.

**Impact:** “Use” can leave dictation unavailable and send the user to dependency troubleshooting. The row also renders **“In use” from selection + installed weights**, without requiring loaded/healthy status ([catalog status](../../src/components/settings/ModelPage.tsx:776)). Runtime repair is placed in the Performance view, making recovery less obvious from Models.

**Fix:** resolve a usable profile-specific environment before activating/reloading an installed model. Reuse an existing verified generation when possible; otherwise run explicit setup. Separate Selected, Installed, Loading and Ready labels. Verify the Qwen → Zipformer → Qwen round trip with retained weights and isolated environments, including offline reuse.

### F06 — P2: captured context has neither a shared token budget nor a truncation notice

**Reproduced slicing behavior; downstream quality impact is code-confirmed.** [SessionContext::inputs](../../src-tauri/src/session.rs:32) gives each of selected text, clipboard and window title `context_size * 2 / 3` **characters**. Its comment says half the window is reserved for instruction/transcript/completion, but the reservation is not enforced across sections.

At a 4,096-token context setting, the probe retained 2,730 characters per section: **8,190 characters total** before instructions, transcript and output. Characters are not tokens, particularly across languages; this observation is not presented as an exact token measurement. Every section is silently prefix-truncated, so relevant text at the end is lost without a marker.

The later request checks prevent many hard overflows, but they reject/fall back rather than adaptively reduce optional context. Cleanup adds all context to every chunk's system prompt; splitting the transcript cannot repair an oversized shared prefix. Commands can additionally include selected text as primary input and context.

**Impact:** enabling more context can cause failed edits/assistant requests or deterministic-cleanup fallback; otherwise the model can answer from an undisclosed partial selection/clipboard.

**Fix:** budget the complete templated request with the runtime tokenizer, reserve output first, prioritize and deduplicate context, and trim lower-priority optional context to fit. Mark omitted/truncated context visibly. Cover large Latin and CJK/Indic inputs, multiple context sources, and important information near the end.

### F07 — P2: Omnilingual rewrites its extracted model on every load

**Code-confirmed; real-weight startup latency was not benchmarked.** [prepare_archive](../../model-runtime/omnilingual_runtime.py:19) hashes the compressed archive, enumerates/decompresses it, and writes a fresh `.part` plus replacement for the ONNX model and tokens every time `OmnilingualModel` is constructed. There is no verified-extracted-file reuse path.

**Trigger:** reload/restart an already-installed Omnilingual model. Even if extracted weights are intact, startup needs write access and enough temporary disk space for another extracted model copy. Cancellation is checked during member copying, but not throughout the initial archive digest/member enumeration.

**Impact:** an installed model can fail to start after disk space shrinks or its directory becomes read-only, and every reload incurs avoidable decompression and writes. This weakens the “download once, then use locally” experience. Integrity checking itself is valuable and should be preserved.

**Fix:** verify extracted model/token digests against a trusted manifest and reuse valid files; re-extract only on missing/corrupt contents. Make repair atomic, preflight staging space, and cover cancellation during verification. Measure cold and repeated load time on the actual weights before making performance claims.

## First-run and model acceptance gaps

These are **not additional reproduced defects**:

1. **The installer is not fully self-contained for offline first use.** It bundles the app, scripts and metadata. Python/dependencies and model weights normally arrive through first-run setup. Existing runtime-pack evidence demonstrates useful isolated installation paths, but this audit did not build and test a release installer with every required artifact already included. The product should distinguish “no terminal required” from “no internet required.”
2. **New models need actual inference acceptance.** The new Omnilingual, Qwen3.5 4B, MiniCPM5 and Gemma integrations have model-free/metadata coverage. Their integration document explicitly says real-weight quality, latency and memory were not benchmarked. Verify download → activation → actual output → restart → offline use, and failure recovery on each supported platform. A model architecture being recognized by llama.cpp is not an accuracy/latency acceptance result.
3. **Physical-platform coverage remains incomplete.** Microphone permissions, global shortcuts, clipboard restoration, Wayland portal approval, external text insertion, Bluetooth changes, installer trust/signing and OS credential-store recovery require actual desktop acceptance. Browser fallback data cannot prove them. Android pairing/TLS/reconnect was source-reviewed but not executed on a phone here; Windows/macOS builds were not rerun on their OSes.
4. **Long meetings remain experimental.** Existing meeting evidence includes a short virtual-route capture, process-kill recovery and synthetic context hierarchy tests. It explicitly does not establish a five-hour physical recording, drift, locked-screen/suspend behavior, or real-model factual recall over five hours. See [existing meeting verification](../verification/meetings-2026-10-24/README.md).
5. **Security scan coverage is partial.** npm was freshly scanned. `cargo-audit` was unavailable, and no fresh Python advisory inventory was run. Historical clean results are not a current all-dependency security certification.

## UX and implementation-integrity review

The UI has a coherent product-specific paper/ink theme, labelled controls, a settings dialog, explicit offline messaging and distinct speech/writing model descriptions. Browser preview exposed the expected setup and disabled-download states. The new catalog clearly marks unavailable adapters instead of pretending they are executable.

The material UX weaknesses found are functional: misleading “In use” readiness, dependency repair being separated from selection, undisclosed context truncation, and retention/encryption guarantees that are not fully met. These matter more than cosmetic changes for the requested “users should not debug the app” goal.

| Dimension                | Evidence and limit                                                                                                                    |
| ------------------------ | ------------------------------------------------------------------------------------------------------------------------------------- |
| Accessibility            | Semantic/label inspection and jsx-a11y lint; no complete screen-reader or measured contrast audit. No WCAG conformance score claimed. |
| Performance              | Build warning: 605.74 kB primary JS; F07 repeated model extraction. No physical low-end startup/frame-time benchmark.                 |
| Theming                  | Existing shared tokens and inspected paper theme are coherent; all accent/high-contrast permutations not visually exercised.          |
| Responsive behavior      | Browser viewport overrides inspected at native minimum/default sizes; native zoom, touch and all text scales not certified.           |
| Implementation integrity | Detector emitted no findings. F05/F06 demonstrate behavior and state-contract gaps that a styling detector cannot detect.             |

A numeric UX health score would imply broader validation than was performed, so none is assigned. Recommended skill work for these UI findings: `$impeccable harden` for readiness/recovery, `$impeccable clarify` for truncation and setup messaging, then `$impeccable polish` after behavior fixes.

## Protections that already work and should be retained

- Managed Python installation uses immutable generations, pinned dependency profiles and atomic activation instead of destroying the previous runtime first.
- Model downloads have pinned integrity metadata; model presence checks reject several incomplete-install states.
- Onboarding distinguishes deferred setup from successful trial evidence and checks runtime/model readiness.
- Cleanup validates output and preserves deterministic text on rejected/truncated model responses.
- Meeting audio/work writes are transactional; processing checkpoints and source references improve recovery and traceability.
- History encryption/recovery, model cancellation, session authority, packaging and network-policy behavior have substantial automated coverage.
- LAN operations have authentication and permission checks; model output proposals are not blindly executed as shell commands.

## Prioritized completion plan

1. Fix **F01–F03** and add focused regression coverage at the shared storage/runtime boundaries.
2. Restore **F04** so exact release checks pass without suppression.
3. Fix **F05–F07**, then verify profile-switching, context-heavy edits and repeated model loads through the GUI.
4. Run a release-artifact acceptance matrix: clean supported machine → install → first setup → first dictation into another app → restart/offline use → cancel/retry download → low-disk/dependency recovery. Record actual loaded model/device and user-visible result.
5. Complete real-weight/new-model and physical long-meeting gates before promoting those capabilities as production-ready.

No fixes were made as part of this audit. The report and evidence are the deliverables.
