# Linux Qwen CPU dependency pack and readiness verification

23 October 2026, Kubuntu x64. The full Windows/macOS/Linux first-run and transcription goal remains incomplete. This checkpoint adds actual Qwen CPU dependency-pack build/install evidence, fixes two demonstrated pack-build blockers and closes a readiness gap. No model weights were downloaded or user model/settings changed. The local partial catalog is not embedded in the app or published.

## Failures and resulting behavior

1. **Vendored metadata blocked real pack creation.** Setuptools 84.2.1 contains one top-level distribution metadata file and 12 vendored metadata files. The scanner treated them all as competing wheel identities and rejected the valid wheel. It now selects the wheel's top-level metadata while retaining bundled license/notice text. Missing or ambiguous top-level identities still fail. This follows the [PyPA wheel layout](https://packaging.python.org/en/latest/specifications/binary-distribution-format/).
2. **The Tokenizers wheel omitted license information.** The pinned 0.22.2 Linux wheel has no License, License-Expression, License-File or notice text. The second real build stopped correctly. The matching upstream commit declares Apache-2.0; a local exact-version notice, pinned source URL and SHA-256 now supply that text. Unknown versions and changed notice bytes still fail. LF checkout rules preserve the hash on Windows. This prepares the inventory; the existing release license review remains required.
3. **A successful package import could falsely establish readiness.** Builder and production Qwen checks now import the concrete processor/model APIs needed by ASR, alongside the actual CPU tensor operation. Quantized Qwen checks also import the required quantization configuration classes. No trained weights are loaded by these checks. A model-free regression rejects a synthetic package whose top-level import succeeds but lazy APIs fail.
4. **Temporary filesystem space was attributed to the root disk.** A real disposable repair exhausted `/tmp`; the worker returned its actionable storage-recovery error. Inspection and a non-allocating regression confirmed that sysinfo 0.33.1 omits tmpfs without its existing `linux-tmpfs` feature. That feature is now enabled. Unix paths are resolved before mount matching so symlinked storage uses the target filesystem. The Linux regression compares admission against `statvfs` for both paths and failed before the fix. Windows keeps its existing drive-prefix handling.

Changed sources are `.gitattributes`, `scripts/runtime-packs.py`, `scripts/test_runtime_packs.py`, `model-runtime/licenses/`, `src-tauri/Cargo.toml`, `src-tauri/src/network_policy.rs` and `src-tauri/src/asr/python_setup.rs`. The existing builder, manifest, atomic generation pointer, bounded probes and installer remain responsible for their operations; no dependency or orchestration framework was added. The opt-in installer fixture now reuses the same CPU-profile procedure rather than duplicating it for Qwen.

## Immutable local artifact

| Item                   | Measured value                                                                                        |
| ---------------------- | ----------------------------------------------------------------------------------------------------- |
| Target/profile         | `x86_64-unknown-linux-gnu`, `qwen-cpu`                                                                |
| Archive                | `speech-runtime-x86_64-unknown-linux-gnu-qwen-cpu.tar.gz`                                             |
| Archive bytes          | 308,489,808                                                                                           |
| Declared payload bytes | 314,999,977, before Python/wheel extraction                                                           |
| SHA-256                | `548fabff5ba36aa7c279eb5e6bd136f54c0415b73d8e1710ec9382ac5ef85f53`                                    |
| Dependency inventory   | 37 wheels; 213 retained license/notice files                                                          |
| Build checks           | Offline wheel installation with sockets denied; `pip check`; concrete Qwen APIs; CPU tensor operation |

`descriptor.json`, `manifest.json`, `build-check.json`, Setuptools metadata evidence and the Tokenizers notice are retained adjacent to this report. The archive remains in the task's local cache. Its descriptor's base commit does not identify the dirty implementation; scoped source hashes in `provenance.json` do.

## Installed-runtime fault and recovery proof

The production Rust worker installs from the verified local pack into isolated application data with offline mode enabled, simulated AppImage and invalid inherited/original Python homes. It checks the receipt, licenses, native imports and CPU operation, cancels an extraction, rejects corrupt/missing caches without a network fallback, retains the incumbent and repairs from the verified cache.

For Qwen, the fixture also damages only its disposable processor module while preserving lazy export definitions. The shallow Torch/Transformers import still passes, but the concrete ASR probe rejects the injected ImportError. A later verified offline generation repairs the environment. The initial fixture removed exports and its overly specific diagnostic assertion failed; that fixture issue was corrected before the passing sequence. Final-source Qwen installation passes in **50.508 seconds**, with the complete fault/recovery sequence passing in **118.63 seconds**. The unchanged Zipformer archive installs in **17.177 seconds**, with its full regression passing in **37.31 seconds**. Both record zero model downloads/network-journal entries. These are local observations under changing desktop load, not setup-time guarantees. Complete results are in `offline-install.json` and `zipformer-regression.json`; the broken-API field is not applicable to Zipformer.

The earlier `/tmp` capacity failure is retained as an observed excerpt, not a complete original log or independent pointer-retention assertion. A relative evidence-path error followed cleanup of only the owned failed fixtures; no unrelated directory was removed. Final checks use explicit evidence paths and a larger filesystem. Neither a zero-entry app journal nor this simulation is a packet capture or actual AppImage installer test.

## Checks, reproduction and limits

The builder/catalog suite passes **8 tests**. Full locked Rust checks pass **983 tests, 27 ignored** across 45 targets; library **639 passed/16 ignored**. All-target Clippy with `-D warnings` passes. Final formatting, artifact/source hashes and separate local pack checks are recorded in `provenance.json`. Ignored tests are not passes. The unchanged frontend retains earlier 371-test/build evidence; no frontend or IPC field changed. The refinement pipeline and its existing 200-trial measurements are unchanged.

Use the [runtime-pack build/catalog commands](../../runtime-packs.md) with profile `qwen`, variant `cpu`, then run the existing ignored installer test alone with `REFLOW_TEST_RUNTIME_PACK_PROFILE=qwen` and its local catalog. Set `TMPDIR` to an existing location with sufficient space for the cache and two complete generations. The local catalog requires `--allow-partial`; release commands still require the complete native matrix.

This proves dependency preparation/recovery on this host. It does not prove trained ASR decoding, microphone permission/recording, shortcut capture, external insertion, accelerator operations, signing/quarantine, another platform or the full clean-installer journey. Native matrix and distribution review, broader disk/crash/process faults, lifecycle-aware old-generation reclamation, 15 allowed Rust advisory warnings, final visual verification and eight substantive writing-quality cases remain open in [the active tracker](../../implementation-progress.md). No release was published.
