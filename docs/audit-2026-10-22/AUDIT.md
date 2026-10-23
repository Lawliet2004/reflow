# Reflow backend, installation, hardware and first-use audit

Audit date: 22 October 2026. Application version: 0.5.0.

Implementation follow-up: [F01–F23 tracker and verification](IMPLEMENTATION.md), [updated 67-scenario acceptance matrix](SCENARIOS.md). Findings/evidence below retain the original audit observations; the scenario matrix reflects the implementation follow-up.

**Assessment: first-run reliability needs further work before broad distribution.** The main risks are unnecessary dependency downloads, CPU/GPU selection before capability discovery, insufficient RAM admission, destructive runtime replacement, and incomplete release gates. Existing tests cover a substantial amount of backend behavior, but passing those tests does not establish a successful installation on a clean customer computer.

This is an audit, not an implementation change. No application code, dependency versions, settings or installed models were changed. The production frontend was rebuilt for verification. This directory contains the report and captured test evidence.

## Scope and confidence

The audited source is the current working tree at commit `148ae33a9deba466dda58874419c9301b56b69db`, including its existing uncommitted changes. It is **not** a claim about a published installer built from another tree. Existing audit documents were used as context; their claims were not treated as current test results.

The review traced onboarding, interpreter discovery, dependency installation, hardware probes, model admission/download/load, runtime replacement/removal, settings recovery, audio imports, history persistence, LAN pairing, platform packaging and release workflows. It ran the checks below and two separate scratch Rust probes against the compiled library. Probes used disposable temporary directories, not the user's application data.

Evidence classifications:

- **Reproduced:** directly observed by a command or scratch probe during this audit.
- **Code-confirmed:** the relevant control flow is present; the customer scenario was not physically exercised.
- **Validation gap:** requires a clean machine, another OS, real microphone/input permissions or a specific hardware/network configuration. It is not being presented as a reproduced defect.

P1 means high priority: a release gate, substantial setup failure, data loss or serious resource mismatch. P2 means a material reliability or behavior issue. P3 means documentation/release expectations. These priorities do not mean every scenario affects every user.

No finite audit can establish every possible scenario. The scenario matrix below records the important classes, what the code does, and what still needs real acceptance testing. No fresh multi-GB dependency installation, microphone recording, text insertion, Android device test or Windows/macOS execution was performed during this audit.

## Checks actually executed

| Check                                             | Result                                                               | Evidence                                                                                        |
| ------------------------------------------------- | -------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `npm run check`                                   | Passed: ESLint, Prettier, TypeScript, 315 tests across 52 files      | [frontend-check.log](evidence/frontend-check.log)                                               |
| `npm run build`                                   | Passed; Vite warns about a 542.56 kB application JS chunk            | [frontend-build.log](evidence/frontend-build.log)                                               |
| Full `cargo test`                                 | **Failed to compile** `long_form_tests`                              | [rust-full-suite.log](evidence/rust-full-suite.log)                                             |
| `cargo test --lib`                                | 566 passed, 14 ignored                                               | [rust-library.log](evidence/rust-library.log)                                                   |
| Every integration target except `long_form_tests` | 326 passed, 8 ignored across 40 targets                              | [integration log](evidence/rust-integration-excluding-broken-target.log)                        |
| `cargo clippy --all-targets -- -D warnings`       | **Failed** on the same stale imports                                 | [rust-clippy.log](evidence/rust-clippy.log)                                                     |
| Python sidecar selftest                           | Passed using the existing managed Python                             | [python-selftest.log](evidence/python-selftest.log)                                             |
| Model-runtime Python unittest discovery           | 39 passed                                                            | [python-tests.log](evidence/python-tests.log)                                                   |
| Packaging/release/shutdown Node tests             | 19 passed, 2 platform-specific skips                                 | [packaging-tests.log](evidence/packaging-tests.log)                                             |
| `npm audit --json`                                | **1 high, 3 moderate dependency entries**, all development tooling   | [npm-security.json](evidence/npm-security.json)                                                 |
| Managed Python `pip check`                        | Passed: no broken declared requirements                              | Existing environment only; not a clean install                                                  |
| Existing managed PyTorch CUDA operation           | Passed: torch 2.13.0+cu130, RTX 2050, tensor sum 1.0 and synchronize | Existing environment only; not recognition or installation verification                         |
| Scratch model/hardware probes                     | Four problematic states confirmed                                    | [source](evidence/model-and-hardware-probe.rs), [output](evidence/model-and-hardware-probe.log) |
| Scratch corrupt-settings probe                    | Original corrupt file overwritten by a normal save                   | [source](evidence/corrupt-settings-probe.rs), [output](evidence/corrupt-settings-probe.log)     |

Audit host: Linux x86_64, Node 22.22.1, Cargo 1.99.0, existing managed Python 3.12. The system `python3` is 3.14.4. Rust dependency advisory scanning was not run because `cargo-audit` is not installed. Python package vulnerability scanning was not run; `pip check` checks declared dependency consistency, not security. These are explicit gaps, not clean security results.

## Findings

### F01 — P1: first model installation can select CPU before CUDA is discovered

**Code-confirmed; resolver behavior reproduced with a fixture.**

On a clean NVIDIA installation, the global torch capability starts false. The startup CUDA probe is inside the branch that already has installed speech weights. Managed setup verifies CUDA in a separate Python process, but does not publish that result into the capability collector. `install_model` calls `resolve_asr_load` before the sidecar install/initialization path has obtained a CUDA result. The resolver therefore returns CPU, and that explicit CPU choice is passed to the model's post-download auto-load. Later CUDA discovery does not automatically replace that loaded CPU model.

**User experience:** several GB of NVIDIA packages download successfully, but the first dictation runs on CPU and appears unexpectedly slow. Restarting or reloading after the probe may change behavior, which makes the problem look intermittent.

Evidence: [startup probe gate](../../src-tauri/src/lib.rs:204), [install selection](../../src-tauri/src/commands/mod.rs:1358), [CPU branch](../../src-tauri/src/profile/resolver.rs:868), [separate setup CUDA verification](../../src-tauri/src/asr/python_setup.rs:802). The fixture reports `device=Cpu` with a driver/GPU present and the speech CUDA result still unprobed. A clean GUI-to-GPU transcription remains required to reproduce the entire customer journey.

**Fix direction:** use one initialize/probe/wait/resolve sequence for startup, installation and reload, and verify the actual loaded device before declaring accelerated setup ready.

### F02 — P1: lightweight speech models require the full heavyweight dependency installation

**Code-confirmed.**

The managed installer receives no selected-model dependency profile. Except on Intel macOS, it installs torch plus the full requirements file, including Qwen, Phonon and Zipformer dependencies, then imports every stack during verification. Even a user choosing the 44 MB CPU Zipformer model needs the full CPU environment or, on a compatible NVIDIA machine, the full CUDA environment. The preflight reserves 5.5 GB or 15.4 GB respectively.

**User experience:** a low-storage/slow-network user selects the smallest model but is blocked by a much larger download. A failure in optional Phonon/torch/torchao dependencies prevents using Zipformer, although Zipformer's own runtime does not need torch. The advice to try Zipformer on an old CPU does not avoid those heavyweight install/import checks.

Evidence: [installation stages](../../src-tauri/src/asr/python_setup.rs:644), [verification imports](../../src-tauri/src/asr/python_setup.rs:779), [storage plan](../../src-tauri/src/asr/setup_plan.rs:63), [separate Zipformer requirements](../../model-runtime/requirements-zipformer.txt:1).

**Fix direction:** install and verify the dependencies for the selected speech engine, with optional expansion later.

### F03 — P1: the real ASR load selector does not enforce available CPU RAM

**Reproduced.**

The CPU branch of `select_asr_load` chooses an installed model, or the requested model when none is installed, without a RAM admission check. The native CPU fallback has the same missing admission constraint. Phonon's CPU and Zipformer's selection paths also do not reject an insufficient RAM budget. The separate profile-preview logic contains RAM budgeting, but it does not make this load path safe.

The scratch fixture with **256 MiB available RAM**, no installed models and a requested Qwen 1.7B returned `1.7b`, CPU, and no error. This is an admission failure; no intentional OOM was performed. The recommendation helper can only suggest 1.7B or 0.6B, so even a machine that cannot support 0.6B is never automatically directed to the lightweight engine.

**User experience:** installation succeeds but model loading causes swapping, long stalls, process termination or an OOM error on small/busy machines.

Evidence: [CPU selection](../../src-tauri/src/profile/resolver.rs:868), [native fallback](../../src-tauri/src/profile/resolver.rs:754), [recommendation](../../src-tauri/src/profile/manifest.rs:494), [probe output](evidence/model-and-hardware-probe.log).

**Fix direction:** enforce memory estimates plus OS/runtime headroom at the actual load boundary, including CPU fallback, and offer a model that fits.

### F04 — P1: dependency repair destroys the working runtime before replacement succeeds

**Code-confirmed.**

Setup replaces `python-base`, deletes `python-runtime`, creates the new environment in its final location, and only then downloads packages and verifies them. Failure/cancellation after ownership is claimed removes the new runtime. No previous verified environment is retained for rollback. A failing GPU tensor check also fails/removes the environment; there is no automatic CPU repair fallback.

The comment about leaving the previous runtime intact before stage 3 is incomplete: stage 2 has already replaced its base interpreter. A cancellation in that interval can leave the old venv coupled to a replaced base. This matters especially when interpreter pins change.

**User experience:** retrying setup to fix one dependency can turn a previously usable installation into an unusable one when the connection drops, disk fills, antivirus interferes or GPU verification fails.

Evidence: [base replacement and runtime deletion](../../src-tauri/src/asr/python_setup.rs:674), [failed/cancelled cleanup](../../src-tauri/src/asr/python_setup.rs:466). The llama runtime already uses staged immutable generations; the Python installer does not.

**Fix direction:** stage and verify a complete new base/environment pair, promote it atomically, and retain the previous usable pair until success.

### F05 — P1: full backend verification and release checks cannot compile

**Reproduced.**

`src-tauri/tests/long_form_tests.rs:16` imports removed `HookMode` and `HotkeyStateMachine` symbols. Full Rust tests and all-target Clippy both fail. CI invokes those commands, and release validation depends on CI.

This is not evidence that the application binary cannot build; the library tests do compile and pass. It is a broken validation/release gate, and the full suite cannot currently establish safety for the shortcut changes in this working tree.

Evidence: [test import](../../src-tauri/tests/long_form_tests.rs:16), [test failure](evidence/rust-full-suite.log), [Clippy failure](evidence/rust-clippy.log).

**Fix direction:** migrate the stale tests to the current gesture/capture contracts and restore the complete gate.

### F06 — P1: the dependency security gate currently fails

**Reproduced.**

`npm audit` reports one high entry for Vite and three moderate entries through Vitest, its mocker and coverage tooling. The captured advisory details include Windows editor-command handling and mock-registration/traversal issues. CI explicitly runs `npm audit --audit-level=high`, so this is another release blocker.

These are development-tool dependencies. This result does **not** establish that a packaged desktop app exposes a Vite server or that the production React bundle is remotely exploitable. Development/preview-server exposure and release validation are the immediate concerns.

Evidence: [full advisory output](evidence/npm-security.json), [CI audit gate](../../.github/workflows/ci.yml:49), [Vite advisory](https://github.com/advisories/GHSA-587h-7wmj-h5pm), [Vitest advisory](https://github.com/advisories/GHSA-qxg6-89mg-rqv8).

**Fix direction:** update the affected toolchain to patched compatible versions and rerun the actual advisory gate.

### F07 — P1: normal saving overwrites a corrupt settings file without preserving a recovery copy

**Reproduced.**

Invalid/unreadable settings cause fallback defaults and a notice saying the original was preserved. Only the newer-schema case sets `write_block_reason`. A later ordinary save replaces the corrupt original with defaults plus the new changes. The scratch probe saved a fallback configuration successfully and found exactly one file: the original bytes were gone.

**User experience:** following a partial write, manual edit or damaged preferences file, the next harmless setting change can permanently remove the only recoverable copy of preferences, modes and dictionary data.

Evidence: [fallback loading](../../src-tauri/src/settings/config.rs:1229), [write protection](../../src-tauri/src/settings/config.rs:1341), [probe output](evidence/corrupt-settings-probe.log).

**Fix direction:** preserve a separate copy of unreadable bytes before permitting replacement, or require an explicit recovery/reset decision.

### F08 — P2: damaged models can be reported as installed, preventing repair downloads

**Reproduced.**

A Qwen directory containing only `reflow-install.json` is considered installed; the receipt is not parsed and the required weight/tokenizer files are not rechecked. Zipformer, Phonon and native checks rely on file presence, so zero-byte files can satisfy the test. The scratch probe confirmed both receipt-only Qwen and zero-byte Zipformer acceptance.

**User experience:** after interrupted copying, disk damage or manual cleanup, onboarding says the model is installed and repeatedly tries loading it. Its setup path reloads installed models instead of redownloading them. The initial downloader's checksum verification does not protect against later damage.

Evidence: [installed check](../../src-tauri/src/model/manager.rs:100), [onboarding installed branch](../../src/components/Onboarding.tsx:277), [probe output](evidence/model-and-hardware-probe.log).

**Fix direction:** validate receipt identity/schema and required nonempty files; provide an explicit verify/repair action, with checksum verification where necessary.

### F09 — P2: dependency preflight does not check the operating-system ABI/version required by wheels

**Code-confirmed; upstream wheel metadata checked.**

The plan checks target architecture but does not gate on macOS version or Linux libc compatibility. For example, PyPI lists the pinned torch 2.13.0 CPython 3.12 Apple Silicon wheel as `macosx_14_0_arm64`; older Apple Silicon macOS installations are still offered the full Qwen/Phonon setup. Published Linux torch wheels carry `manylinux_2_28` tags. Architecture alone is not proof that a pinned binary can run.

**User experience:** Python download and environment creation succeed, then pip reports no compatible distribution or native imports fail. Native runtime minimum OS/CPU requirements also need a published support floor. Packaged-app constraints may prevent some old Linux systems from reaching this stage at all; that does not make the setup planner's compatibility check complete.

Evidence: [setup plan](../../src-tauri/src/asr/setup_plan.rs:75), [torch metadata](https://pypi.org/pypi/torch/2.13.0/json), [torch release files](https://pypi.org/project/torch/2.13.0/). Actual older-OS execution was not tested.

**Fix direction:** publish and enforce the effective OS/libc/CPU support matrix before downloading, with lightweight alternatives where supported.

### F10 — P2: package flavor ignores the user's selected CPU device and selected model

**Code-confirmed.**

`resolved_torch_variant` uses hardware capabilities and a debug environment override; it does not receive `settings.asr.device` or model choice. An NVIDIA user intentionally choosing CPU-only still receives CUDA packages. This also affects Zipformer, whose runtime is CPU-only.

**User experience:** a deliberate CPU choice does not reduce download/storage requirements and may encounter GPU-package setup failures that the user intended to avoid.

Evidence: [variant selection](../../src-tauri/src/asr/python_setup.rs:558), [hardware routing](../../src-tauri/src/asr/setup_plan.rs:15).

**Fix direction:** include intended engine and execution device in the installation plan, and explain the actual package set before consent.

### F11 — P2: changing the compute-backend selector does not reload the resident ASR model

**Code-confirmed.**

The selector only saves an ASR setting through `change`. The backend schedules reloads for model/runtime/interpreter changes, not device changes. Precision and preset controls explicitly call `reloadModel`, demonstrating a different behavior for adjacent settings.

**User experience:** Settings says CPU-only or GPU-only, but the resident model continues on the previous device until a manual reload/restart. A user trying to free GPU memory can see no immediate effect.

Evidence: [compute selector](../../src/components/settings/ModelPage.tsx:1049), [change helper](../../src/components/settings/ModelPage.tsx:249), [backend reload conditions](../../src-tauri/src/commands/mod.rs:670), [precision reload](../../src/components/settings/ModelPage.tsx:342).

**Fix direction:** apply device changes at a safe reload boundary or clearly mark them pending until reload.

### F12 — P2: “GPU only” is not enforced as a strict execution requirement

**Code-confirmed.**

The real resolver treats an unavailable CUDA device as CPU even when `requested_device` is `cuda`. If nothing fits GPU memory it also returns CPU. The Python layer has a strict-device check, but it receives the resolver's rewritten CPU choice rather than the original user requirement. The native path similarly falls back to CPU.

**User experience:** GPU-only can still run on CPU, contrary to the control's label. This can produce severe unexpected latency. Some fallback paths explain their decision, but an explanation does not make the “only” contract accurate.

Evidence: [selector label](../../src/components/settings/ModelPage.tsx:1071), [effective device](../../src-tauri/src/profile/resolver.rs:823), [CPU fallback](../../src-tauri/src/profile/resolver.rs:979), [Python strict check](../../model-runtime/qwen3_asr_runtime.py:673).

**Fix direction:** reject unavailable forced execution or rename the choice to a preference and show the fallback consistently.

### F13 — P2: an inherited virtual environment can shadow a successfully installed managed runtime

**Code-confirmed.**

Interpreter discovery prefers `VIRTUAL_ENV` ahead of the managed interpreter. `runtime_overridden` only recognizes a saved interpreter path or `REFLOW_PYTHON`, so setup can install a managed environment and claim success while the sidecar still chooses an activated environment with missing/incompatible packages.

**User experience:** common in development or launches from an activated shell: setup downloads everything, then Reflow still reports missing speech dependencies. Repeating setup targets the environment that is not actually selected.

Evidence: [discovery priority](../../src-tauri/src/asr/sidecar.rs:327), [override detection](../../src-tauri/src/asr/python_setup.rs:335).

**Fix direction:** display the effective interpreter and make installation target/discovery priority agree, including inherited environments.

### F14 — P2: an old runtime receipt is treated as ready without checking current requirements

**Code-confirmed.**

Managed readiness is interpreter existence plus any parseable runtime receipt. The receipt records Python build/version/flavor/date but no requirements digest or installed package versions, and readiness does not compare it with current pins. `prepareSpeechRuntime` skips setup when that marker is present unless a missing-dependency failure has already been observed.

**User experience:** after an app upgrade or hardware/driver change, old dependencies can remain “ready” until loading fails. Importable but incompatible versions may not produce the missing-dependency classification that triggers repair.

Evidence: [receipt/status](../../src-tauri/src/asr/python_setup.rs:348), [frontend reuse rule](../../src/services/speechSetup.ts:14).

**Fix direction:** version the environment against its requirement set and target, with a non-destructive compatibility check and upgrade path.

### F15 — P2: model-weight downloads have no user-facing cancel operation

**Code-confirmed.**

Dependency setup has a cancellation command. Speech-weight installation instead starts a background model-installer and exposes downloading status without a corresponding cancel-download command. The onboarding cancel button is only shown while dependency progress exists; it disappears during the separate weight transfer. The model-load deadline is intentionally suspended during downloading.

**User experience:** after committing to a 1.6/4.1 GB download, the user cannot directly cancel that transfer from its progress UI. Airplane mode or closing the process can stop it, but neither provides the expected scoped cancel action.

Evidence: [onboarding phase controls](../../src/components/Onboarding.tsx:648), [model installer thread](../../model-runtime/qwen3_asr_runtime.py:1991), [sidecar install control](../../src-tauri/src/asr/sidecar.rs:1238).

**Fix direction:** give weight downloads their own operation ID, cancellation signal and resumable terminal state.

### F16 — P2: pip traffic bypasses the app's advertised host allowlist and download journal

**Code-confirmed policy/observability gap.**

Pip is a subprocess, so its requests do not pass through `network_policy::send_download` or the Python sidecar HTTP guard. Its PyPI/PyTorch package bytes are not recorded in the app journal. The package stages also inherit pip configuration/environment; only the torch stage specifies an explicit package index. Requirements pin direct versions but not the entire transitive resolution or artifact hashes.

**User experience:** the network journal omits much of the initial setup traffic, and network/proxy/index behavior can differ from artifact downloads. This conflicts with documentation saying each HTTPS request is restricted to GitHub/Hugging Face hosts. This is not a claim that pip lacks TLS or that setup runs without the user's download consent. Mid-install Airplane-mode polling does kill pip; that safeguard is present.

Evidence: [pip arguments](../../src-tauri/src/asr/python_setup.rs:575), [process launch](../../src-tauri/src/asr/python_setup.rs:1109), [host allowlist/journal](../../src-tauri/src/network_policy.rs:85), [documented contract](../../docs/offline-transfer.md:10).

**Fix direction:** define the additional package-host policy, account for package downloads, isolate unintended pip configuration where appropriate, and lock/hash platform dependency sets.

### F17 — P2: removing the Python runtime can race with a newly starting session

**Code-confirmed race; not stress-reproduced.**

Removal checks the state, calculates sizes, shuts down the ASR engine and deletes runtime directories, but does not acquire `session_operation`. Dictation/file work can begin between the state check and shutdown. Setup itself correctly acquires that operation guard; removal does not.

**User experience:** a hotkey press racing “Remove speech runtime” can begin a session whose runtime is then stopped/deleted, losing that session's recognition or producing an error.

Evidence: [runtime removal](../../src-tauri/src/asr/python_setup.rs:528), [guarded setup for comparison](../../src-tauri/src/asr/python_setup.rs:412).

**Fix direction:** serialize admission, stop and removal under the same operation ownership contract.

### F18 — P2: disk preflight is not a complete peak-space calculation

**Code-confirmed limitation.**

The dependency estimate is a fixed CPU/CUDA number and excludes the selected model; model capacity is checked later. Pip temporary extraction uses its inherited temporary directory, which may be on a different filesystem from app data. Runtime archive checks primarily budget compressed download bytes; extraction, staging and retained generations can need more. Offline model import hashes/copies into staging without an up-front disk budget.

**User experience:** a machine can pass the displayed check, spend substantial time downloading, then fail during extraction, model installation or import. A large data partition does not imply enough space on a small system/temp partition.

Evidence: [dependency estimate](../../src-tauri/src/asr/setup_plan.rs:63), [setup preflight](../../src-tauri/src/asr/python_setup.rs:654), [runtime preflight](../../src-tauri/src/rewrite/runtime_install.rs:502), [import staging](../../src-tauri/src/transfer.rs:415).

**Fix direction:** estimate the combined selected-engine/model peak and check every filesystem used for cache, temp, environment and staging; handle uncertain estimates explicitly.

### F19 — P2: file-transcription cancellation cannot interrupt decoding or active inference

**Code-confirmed.**

Cancel only sets the file-job flag. `decode_file` receives no cancellation token and must finish or error before that flag is checked. During `stop_stream`, cancellation is checked after inference returns; the file cancel handler does not trigger the sidecar's existing out-of-band inference-cancellation signal.

**User experience:** Cancel can leave the app in Processing, block dictation and continue using CPU/RAM for a long time on a large/compressed file or slow CPU model.

Evidence: [decode task](../../src-tauri/src/file_jobs.rs:227), [inference loop](../../src-tauri/src/file_jobs.rs:65), [cancel handler](../../src-tauri/src/file_jobs.rs:290), [decoder](../../src-tauri/src/audio/file_decode.rs:54).

**Fix direction:** carry cancellation through decode and active ASR work, and await cleanup before releasing the session.

### F20 — P2: two-hour imports buffer the full decoded recording in RAM

**Code-confirmed; memory calculation, not an OOM benchmark.**

Although input packets are decoded incrementally, their resampled output is accumulated into one `Vec<f32>` before recognition starts. The two-hour cap permits 115,200,000 samples: **460.8 MB of sample payload alone**, excluding capacity growth, decoder buffers, retained audio encoding and the speech model. Compressed input can stay below the 100 MB confirmation threshold while expanding into a large buffer.

**User experience:** long MP3/M4A imports on memory-constrained hardware can stall or terminate the app despite a small file size and a supported duration.

Evidence: [sample cap](../../src-tauri/src/audio/file_decode.rs:50), [accumulation](../../src-tauri/src/audio/file_decode.rs:78), [whole-file handoff](../../src-tauri/src/file_jobs.rs:227).

**Fix direction:** decode/transcribe bounded segments with backpressure, and budget decoded duration/memory rather than only compressed bytes.

### F21 — P2: file-import persistence can partially succeed while the job reports failure

**Code-confirmed.**

History-row insertion, subtitle-segment saving and retained-audio saving are separate fallible operations. A failure after the row is inserted returns a failed job without rolling the row back or returning its ID as a partial success.

**User experience:** disk-full/database/keyring failures near completion can show “failed” even though a transcript already exists. Retrying can create duplicates; subtitle/audio availability may differ from the apparent result.

Evidence: [sequential writes](../../src-tauri/src/file_jobs.rs:282), [job result handling](../../src-tauri/src/file_jobs.rs:198).

**Fix direction:** make the necessary database writes transactional, and represent optional audio failure as a successful transcript with an explicit warning.

### F22 — P2: phone-pairing QR can choose an unreachable virtual/VPN interface

**Code-confirmed.**

LAN detection collects RFC1918 addresses from all interfaces, sorts them as strings, and the QR uses the first non-loopback address. There is no route-to-phone check, interface choice or exclusion for Docker/VM/VPN addresses. A `172.*` virtual adapter can sort before the `192.168.*` Wi-Fi address the phone can reach.

**User experience:** scanning a valid certificate-pinned QR still times out. A firewall, guest-Wi-Fi isolation or different subnet can cause the same symptom. The latter are network acceptance cases, not proof of this selection defect.

Evidence: [address collection](../../src-tauri/src/api/net.rs:6), [QR host choice](../../src-tauri/src/api/server.rs:50).

**Fix direction:** expose interface selection and connection diagnostics, preferring the appropriate LAN route instead of lexical address order.

### F23 — P3: installer/release documentation describes behavior that the workflow no longer implements

**Code-confirmed.**

README says tags produce a draft release that requires separate publication, and macOS installers remain gated. The workflow includes both macOS preview installers and automatically runs `gh release edit --draft=false --latest` when all jobs succeed. README also still leads with version 0.4.0 while package manifests are 0.5.0.

**User/maintainer experience:** users can infer stronger platform support or a different release-review step than actually exists. The automatic publication behavior is a policy mismatch; this audit does not change it or presume that the user wants a different policy.

Evidence: [README](../../README.md:5), [macOS matrix](../../.github/workflows/release.yml:79), [automatic publication](../../.github/workflows/release.yml:235).

**Fix direction:** align the published support statement, preview labels, signing limitations and actual release approval/publication policy.

## First-user scenario matrix

“Handled in code” below means safeguards were found; it does not imply physical testing on every listed platform.

| #   | Scenario                                                      | Current result                                                                                                                                       | Acceptance                    |
| --- | ------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------- |
| 1   | Clean Windows/Linux installer, no Python/Node/Rust            | Managed installer remains independent of developer Python/Node/Rust; isolated Linux dependency setup passed. Packaged clean machine pending.         | [A](SCENARIOS.md#procedure-a) |
| 2   | User clones source expecting an immediately runnable app      | Source build prerequisites remain documented; frontend/backend gates pass on this host. Fresh source machine pending.                                | [A](SCENARIOS.md#procedure-a) |
| 3   | Existing managed environment on this Linux RTX 2050           | Existing interpreter used read-only for model-free tests; fresh isolated Zipformer setup separately passed. No new microphone/GPU recognition claim. | [B](SCENARIOS.md#procedure-b) |
| 4   | Clean supported NVIDIA machine, first model                   | Discovery completes before first-install resolution; regression selects CUDA without installed weights. Clean GPU installer pending.                 | [B](SCENARIOS.md#procedure-b) |
| 5   | User chooses 44 MB Zipformer to minimize download             | Fresh isolated torch-free Zipformer dependency installation and cancelled repair passed; recognition pending.                                        | [A](SCENARIOS.md#procedure-a) |
| 6   | NVIDIA machine deliberately configured CPU-only               | Explicit CPU selects CPU package locks, including on NVIDIA hardware. Physical fresh install pending.                                                | [B](SCENARIOS.md#procedure-b) |
| 7   | Intel/AMD integrated GPU                                      | CPU remains the supported managed route; no AMD/XPU acceleration promise. Physical latency pending.                                                  | [B](SCENARIOS.md#procedure-b) |
| 8   | Apple Silicon GPU/MPS                                         | MPS remains unsupported by this speech runtime; Apple Silicon uses CPU. Physical acceptance pending.                                                 | [B](SCENARIOS.md#procedure-b) |
| 9   | Intel Mac                                                     | Only Zipformer automatic profile is offered. Target wheel lock resolved; physical Mac install pending.                                               | [A](SCENARIOS.md#procedure-a) |
| 10  | Apple Silicon on older macOS                                  | macOS version floor checked before large downloads; Apple Silicon requires 14+. Physical boundary tests pending.                                     | [C](SCENARIOS.md#procedure-c) |
| 11  | Legacy x86 NVIDIA with suitable driver                        | CUDA 12.6 compatibility profile retained and locked. Legacy GPU kernels/recognition pending.                                                         | [B](SCENARIOS.md#procedure-b) |
| 12  | Driver too old or compute capability unknown                  | Unverified/incompatible driver chooses CPU packages with explanation; forced GPU load fails visibly. Physical legacy driver pending.                 | [B](SCENARIOS.md#procedure-b) |
| 13  | Driver looks compatible but real CUDA operation fails         | Failed verification cannot promote staged generation; previous base and venv remain selected. Real CUDA failure pending.                             | [D](SCENARIOS.md#procedure-d) |
| 14  | Multiple NVIDIA adapters / remapped `CUDA_VISIBLE_DEVICES`    | Existing unsafe-mapping rejection preserved; cuda:0-compatible package selection. Remapped/multiple adapter acceptance pending.                      | [B](SCENARIOS.md#procedure-b) |
| 15  | Small GPU occupied by games/browser/display                   | Live VRAM admission remains active; native launch also rechecks budget and strict GPU cannot fall back. WDDM stress pending.                         | [B](SCENARIOS.md#procedure-b) |
| 16  | Very little available system RAM                              | All engine selectors and CPU load/fallback paths reject insufficient available RAM; Zipformer offered. Physical RSS pending.                         | [E](SCENARIOS.md#procedure-e) |
| 17  | Old x86 CPU without AVX2                                      | Heavy setup blocks without AVX2; Zipformer has a separate lock/import path. Non-AVX2 wheel execution pending.                                        | [C](SCENARIOS.md#procedure-c) |
| 18  | Linux ARM64 desktop                                           | ARM64 target/profile locks resolved; native libraries and recognition on physical ARM64 pending.                                                     | [C](SCENARIOS.md#procedure-c) |
| 19  | Jetson or other specialized ARM NVIDIA stack                  | Generic ARM locks do not establish JetPack compatibility. Specialized stack remains acceptance pending.                                              | [C](SCENARIOS.md#procedure-c) |
| 20  | Windows ARM / 32-bit OS                                       | No automatic managed/native target added; unsupported architectures stay blocked. Emulation unverified.                                              | [C](SCENARIOS.md#procedure-c) |
| 21  | Musl Linux / old glibc / unusual distro                       | Musl rejected; glibc floors checked before download. Distro GUI ABI acceptance remains separate.                                                     | [C](SCENARIOS.md#procedure-c) |
| 22  | Not enough data-volume space before setup                     | Peak budget includes dependency generation/cache/unpack and selected model plus margin. Full disk injection pending.                                 | [F](SCENARIOS.md#procedure-f) |
| 23  | Dependencies fit but selected model does not                  | Combined model/dependency peak is checked before setup; later download also checks its volume. Boundary acceptance pending.                          | [F](SCENARIOS.md#procedure-f) |
| 24  | Temp/system disk full while app-data disk has room            | Pip temp is under app cache on checked volume; extraction/generation also stays there. Real full-volume fault pending.                               | [F](SCENARIOS.md#procedure-f) |
| 25  | Deep Windows profile / long paths disabled                    | Existing long-path safeguard preserved; generation paths are longer. Windows long-path boundaries pending.                                           | [F](SCENARIOS.md#procedure-f) |
| 26  | Read-only app data, portable directory or noexec mount        | Errors preserve prior runtime generation; no permission/noexec bypass. Physical error-message acceptance pending.                                    | [F](SCENARIOS.md#procedure-f) |
| 27  | Internet absent at first launch                               | Consent remains closed by default; no setup download without enabling it. Offline policy tests pass.                                                 | [G](SCENARIOS.md#procedure-g) |
| 28  | Airplane mode enabled mid dependency install                  | Worker checks policy and kills pip process tree; pip body guard also rereads policy. Real platform transfer interruption pending.                    | [G](SCENARIOS.md#procedure-g) |
| 29  | Airplane mode enabled mid artifact transfer                   | Guarded artifact reads stop and keep partials; policy tests pass. Real stalled transfer pending.                                                     | [G](SCENARIOS.md#procedure-g) |
| 30  | Captive portal, corporate TLS proxy, authentication proxy     | Fixed HTTPS destinations/config isolation preserve TLS checks and system proxy support. Corporate/captive acceptance pending.                        | [G](SCENARIOS.md#procedure-g) |
| 31  | GitHub/PyPI/PyTorch/Hugging Face blocked separately           | Stage failure cannot replace incumbent; package/artifact destinations are guarded and journaled. Per-host fault injection pending.                   | [G](SCENARIOS.md#procedure-g) |
| 32  | HTTP 429/5xx/transient connection error                       | Bounded retries retained; native body interruptions resume Range transfers with validated offsets. Real fault server matrix pending.                 | [G](SCENARIOS.md#procedure-g) |
| 33  | Cancel during interpreter extraction or hashing               | Extraction checks each entry; weight checks check chunks. A large archive entry or synchronous archive hash can delay cancel.                        | [D](SCENARIOS.md#procedure-d) |
| 34  | Cancel while pip downloads                                    | Child/process-group cancellation retained; setup failure/cancel removes staged generation only. Platform tree tests pending.                         | [D](SCENARIOS.md#procedure-d) |
| 35  | Cancel while model weights download                           | Onboarding/model settings expose model-scoped cancellation. Native ownership and sidecar event preserve partials; real stall pending.                | [D](SCENARIOS.md#procedure-d) |
| 36  | Quit/power loss during dependency repair                      | Active pointer changes only after complete verification; interrupted pointer temp ignored. Unit rollback passes; power loss pending.                 | [D](SCENARIOS.md#procedure-d) |
| 37  | Retry with a completed download cache                         | SHA-256-verified bootstrap cache reuse exercised in fresh setup; second repair uses package cache. Traffic savings not benchmarked.                  | [D](SCENARIOS.md#procedure-d) |
| 38  | Interrupted partial wheel download, then restart entire setup | Completed pip cache survives; partial wheels killed with pip are not guaranteed reusable across processes. No stronger resume claim.                 | [D](SCENARIOS.md#procedure-d) |
| 39  | App update with an older managed environment                  | Receipt matches target/profile/flavor/digest and actual metadata/prefix before readiness. Legacy update acceptance pending.                          | [H](SCENARIOS.md#procedure-h) |
| 40  | Activated broken virtual environment                          | Managed generation precedes inherited VIRTUAL_ENV; explicit overrides still lead. Packaged broken-venv launch pending.                               | [H](SCENARIOS.md#procedure-h) |
| 41  | Explicit broken interpreter                                   | Override remains deliberate and automatic setup will not silently replace it. Clear/repair override and reload; UI acceptance pending.               | [H](SCENARIOS.md#procedure-h) |
| 42  | Antivirus/quarantine locks Python or runtime DLL              | Failed staged work leaves incumbent selected; OS locks may also prevent cleanup/removal. Windows antivirus acceptance pending.                       | [D](SCENARIOS.md#procedure-d) |
| 43  | Downloaded model later missing/truncated files                | Receipt cannot bypass file completeness; nonempty structure required and verify/repair quarantines bad pinned weights. Tests pass.                   | [H](SCENARIOS.md#procedure-h) |
| 44  | Fresh offline/air-gapped machine imports valid weights        | Existing checksum/metadata import safeguards retained. Matching runtime still required separately; no air-gap runtime bundler added.                 | [H](SCENARIOS.md#procedure-h) |
| 45  | No microphone / permission denied / unplugged device          | Existing permission/device errors and Record-button recovery preserved. Physical microphone acceptance pending.                                      | [I](SCENARIOS.md#procedure-i) |
| 46  | Linux selected mic but `pw-record` missing                    | Existing explicit selected-PipeWire error and System default fallback preserved; physical backend test pending.                                      | [I](SCENARIOS.md#procedure-i) |
| 47  | PulseAudio rather than PipeWire                               | Existing backend preference/fallback behavior retained; physical PulseAudio acceptance pending.                                                      | [I](SCENARIOS.md#procedure-i) |
| 48  | Bluetooth profile switches/disconnects mid-recording          | Existing device-error/session behavior retained; physical Bluetooth disconnect/reconnect pending.                                                    | [I](SCENARIOS.md#procedure-i) |
| 49  | Wayland shortcut portal missing/denied                        | Native portal status and Record fallback preserved; gesture tests now compile. Real desktop permission matrix pending.                               | [I](SCENARIOS.md#procedure-i) |
| 50  | Wayland automatic-paste permission missing/denied             | Existing paste portal status/fallback preserved; permission denied acceptance pending.                                                               | [I](SCENARIOS.md#procedure-i) |
| 51  | Shortcut collides with IME/desktop/app                        | Capture/current gesture coverage passes; actual IME/desktop collision and native registration pending.                                               | [I](SCENARIOS.md#procedure-i) |
| 52  | Windows target app elevated, secure field or clipboard busy   | Existing clipboard fallback preserved; real elevated/secure app/focus/clipboard acceptance pending.                                                  | [I](SCENARIOS.md#procedure-i) |
| 53  | macOS mic/Accessibility permission missing                    | macOS stays preview; no physical permission or insertion claims. Acceptance pending.                                                                 | [I](SCENARIOS.md#procedure-i) |
| 54  | Recognition test passes but external insertion fails          | Recognition and insertion remain separate acceptance gates; complete external paste journey pending.                                                 | [I](SCENARIOS.md#procedure-i) |
| 55  | Compute device changed after model ready                      | Device change schedules safe resident reload; forced GPU cannot silently stay CPU. Real switch/session acceptance pending.                           | [B](SCENARIOS.md#procedure-b) |
| 56  | Runtime removal races a hotkey press                          | Removal holds session admission before state check and deletion, excluding setup. Packaged hotkey stress pending.                                    | [D](SCENARIOS.md#procedure-d) |
| 57  | Two-hour compressed file on low-RAM system                    | Bounded segments and two-slot channel replace whole-file decode; retained audio streams to SQLite. Two-hour RSS pending.                             | [E](SCENARIOS.md#procedure-e) |
| 58  | Cancel a long audio import                                    | Packet decode checks and out-of-band inference signal cancel active work. Decoder/native/sidecar regressions pass; codecs pending.                   | [E](SCENARIOS.md#procedure-e) |
| 59  | Disk/keyring/database error after file transcription          | Transcript/segments transactional; optional retained audio failure returns saved result plus warning. Injected storage tests pass.                   | [E](SCENARIOS.md#procedure-e) |
| 60  | Corrupt settings followed by normal save                      | Exact unreadable bytes durably backed up before normal save; invalid UTF-8 regression passes. Filesystem failure acceptance pending.                 | [H](SCENARIOS.md#procedure-h) |
| 61  | Encrypted history but missing key / corrupt DB                | Privacy/key recovery protections preserved; audio chunks participate in encryption and retention. OS keyring matrix pending.                         | [H](SCENARIOS.md#procedure-h) |
| 62  | Docker/VPN/VM adapters present during phone pairing           | Routed ordinary interfaces rank ahead of common virtual networks; user can select a local address. Ranking test passes, phone pending.               | [J](SCENARIOS.md#procedure-j) |
| 63  | Phone on isolated guest Wi-Fi / firewall blocks listener      | UI explains same Wi-Fi, VPN/firewall and guest isolation; phone/network acceptance pending.                                                          | [J](SCENARIOS.md#procedure-j) |
| 64  | API port already occupied                                     | UI shows listener failure guidance with port/firewall/toggle recovery; physical occupied-port acceptance pending.                                    | [J](SCENARIOS.md#procedure-j) |
| 65  | Revoked phone token / switched listener                       | Existing authorization/revocation/listener/fingerprint tests pass; physical Android stream lifecycle pending.                                        | [J](SCENARIOS.md#procedure-j) |
| 66  | New version tag pushed from this tree                         | Rust/npm gates restored; README now describes automatic complete publication. No tag pushed or release generated.                                    | [K](SCENARIOS.md#procedure-k) |
| 67  | Unsigned Windows installer / unsigned macOS preview           | README describes conditional signing and macOS preview; physical clean-machine trust prompts pending.                                                | [K](SCENARIOS.md#procedure-k) |

## Existing protections verified in code and tests

The following should be preserved during fixes:

- Download consent defaults closed; interpreter and model downloads use pinned artifact identities/checksums.
- Managed package setup uses binary-only installation, does not require a compiler, checks a real CUDA operation and supports child-process cancellation.
- The current tested environment is internally dependency-consistent and can execute a CUDA operation.
- Speech uses local loading with remote code disabled. Model import rejects unknown weight hashes and bounds metadata.
- Audio/session ownership, cancellation, retention and privacy recovery have substantial automated coverage. The current library and compilable integration targets pass.
- LAN uses HTTPS/WSS with certificate fingerprints, hashed tokens, restricted default phone permissions, revocation rechecks and bounded messages/body sizes. No authentication bypass was confirmed in this review.
- Clipboard fallback, desktop portal status and microphone/recognition test paths exist. The application does not silently substitute mock recognition when the real speech process fails.
- A first-time user can select **Set up later**. A broken dependency installation does not force permanent confinement inside onboarding.

## Remaining release-validation gaps

1. **A complete clean install has not been established.** The earlier [first-run setup verification](../first-run-setup-verification.md:36) itself says the fresh CPU wheel download timed out and the NVIDIA run was stopped before completion. This audit did not repeat that large download. A successful existing environment is not equivalent evidence.
2. **Installer acceptance must start with no developer environment.** Use fresh Windows and Linux machines/VMs with no system Python, no activated venv, no existing model/cache and no source checkout. Verify install → dependency setup → model → recognition → external insertion → restart → offline use.
3. **Hardware fixtures are not kernel compatibility tests.** RTX 2050 tensor execution does not verify legacy NVIDIA quantization, ARM/Jetson, AMD/Intel CPU performance, Apple Silicon/Intel Mac, non-AVX2 CPUs or WDDM spill behavior.
4. **Test interruption and repair on real installers.** Drop connectivity and fill relevant disks at every stage; cancel, quit, reboot, retry and upgrade. Verify old usable environments survive failures and no orphan process continues downloading.
5. **Physical audio and insertion are separate acceptance gates.** Test headset/default/Bluetooth changes, Linux audio backends, focus, shortcuts, permission denial/revocation, elevated Windows apps, terminals, password fields and clipboard contention.
6. **Security scanning remains incomplete.** Restore the npm gate, run RustSec on the exact lockfile, scan the installed Python set, and retain a package inventory for each platform/flavor. No clean result is claimed for scans not executed.
7. **Language and accuracy need real audio.** English-only Zipformer/Phonon should be tested against language selection and multilingual expectations. Real English/Hindi corpus accuracy, native-runtime accuracy and cold/warm latency were not measured here.
8. **Android and macOS remain separate acceptance work.** Code/tests do not establish permission, TLS pinning, network recovery, background lifecycle or actual paste behavior on physical devices.

## Suggested order for the next implementation pass

1. Restore full Rust and dependency-security gates: F05/F06.
2. Make the selected model/device determine dependencies and admission: F01/F02/F03/F09/F10/F11/F12.
3. Make runtime setup/repair transactional, and align readiness/interpreter identity: F04/F08/F13/F14/F17/F18.
4. Preserve corrupt settings and correct cancellation/persistence: F07/F15/F19/F20/F21.
5. Complete download-policy visibility and first-run recovery/phone diagnostics: F16/F22/F23.
6. Run the clean-machine and hardware acceptance matrix against the actual installer bytes, record the loaded backend and measured latency, then assess distribution readiness again.

No fixes have been applied. The user can review findings by ID before selecting the next changes.
