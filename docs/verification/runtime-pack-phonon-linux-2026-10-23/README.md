# Linux Phonon dependency readiness and actual decoding

23 October 2026, Kubuntu x64. This checkpoint fixes false readiness and verifies a fresh model-free dependency pack through the production installer and actual Rust ASR sidecar. Existing weights were copied locally and their source archive stayed unchanged. No model weights were downloaded, settings changed or release published. Windows/macOS/Linux remain in scope; the user authorized CI builds and deferred physical-device tests.

## Demonstrated readiness bug

Top-level `fermion`, `zstandard` and Transformers imports could pass even when the concrete Parakeet model API was unusable. The builder, installed-runtime health check and sidecar initialization now verify the actual Fermion CPU engine plus `ParakeetForTDT` and `ParakeetTDTConfig`. Initialization reuses the existing main-thread dependency check. Errors direct users to repair speech dependencies. The packed native CPU library is not required by this check because upstream supports an fp32 path.

Two negative regressions failed before the fix: the builder accepted a synthetic broken API and sidecar dependency verification did not raise. The final builder suite passes eight tests; the model-runtime suite passes 45. The opt-in installer fixture also damages the real disposable Parakeet module while retaining lazy export definitions. Top-level Torch/Transformers imports still succeed, but the production probe rejects the injected ImportError; verified offline repair restores a healthy generation. The shared Qwen fixture remains covered.

Changed implementation/test files are `model-runtime/phonon_runtime.py`, `model-runtime/qwen3_asr_runtime.py`, `model-runtime/test_phonon.py`, `scripts/runtime-packs.py`, `scripts/test_runtime_packs.py`, `src-tauri/src/asr/python_setup.rs` and `src-tauri/tests/phonon_live.rs`. Existing setup and sidecar boundaries remain responsible for the work.

## Pack and installation evidence

The `x86_64-unknown-linux-gnu` Phonon CPU archive is **349,482,063 bytes**, SHA-256 `80a6eb835b574612a6001d4c677c7e16582416c09c2084c429676b859adecc7b`. It contains pinned Python 3.12.15 and binary wheels. The builder proves socket-denied wheel installation, `pip check`, concrete ASR imports and a CPU tensor operation. The adjacent descriptor, manifest, build checks and license inventory summary bind the inputs. Distribution review remains required.

The real production installer passes offline with a simulated AppImage environment and invalid inherited/original Python homes: **55.251 seconds** installation, **126.73 seconds** complete fault/recovery sequence. Cancellation, corrupt/missing cache, broken lazy API and repair pass, preserving the incumbent with zero model downloads/network-journal entries. Final-source Qwen and Zipformer regressions also pass: installation 53.209/17.677 seconds, complete sequences 122.38/37.76 seconds. A zero-entry app journal is not a packet capture.

`REFLOW_TEST_RUNTIME_PACK_KEEP=1` retains only the isolated test environment for follow-on recognition. The fresh-runtime receipt was checked before use. After evidence capture, the matching owned scratch tree was removed; `scratch-cleanup.json` records that check. This is not production generation garbage collection. The local partial catalog is neither embedded nor published.

## Actual ASR evidence

Installed FFmpeg's local Flite voice generated the 4.605-second, ten-word fixture without a voice download:

> The application should work offline. Please keep my recordings private.

A fresh environment received a local copy of the installed archive/config/notices with no unpacked model cache before its first load. The actual Rust sidecar received PCM audio, returned every reference word, reported the expected backend/precision, rejected Hindi requests and unloaded successfully.

| Run                                     | Interpreter                    | Load ms | Stop/decode ms | Reference words |
| --------------------------------------- | ------------------------------ | ------: | -------------: | --------------- |
| Packed int8, AVX2                       | Fresh dependency pack          |   34644 |            227 | 10/10           |
| Forced supported fp32                   | Same fresh environment         |   23068 |            383 | 10/10           |
| 48 kHz stereo, silent left/speech right | Existing installed interpreter |   11047 |            202 | 10/10           |

The stereo case exercises actual downmix/resampling; it is separate from fresh-package acceptance. The forced fp32 run proves that selected path, not automatic fallback on an unsupported CPU. Load measurements include different cache states and do not demonstrate a speed improvement. These are synthetic recognition samples, not an accuracy corpus or microphone/shortcut/insertion acceptance. CUDA was not exercised in this checkpoint.

Reproduce the dependency check using the [runtime-pack instructions](../../runtime-packs.md) with profile `phonon`, variant `cpu`, and sufficient temporary filesystem space. Run the opt-in installer test alone. For real sidecar decoding, provide existing weights through `REFLOW_PHONON_MODEL_DIR`, a fixture through `REFLOW_PHONON_AUDIO`, the reference through `REFLOW_PHONON_REFERENCE`, and `REFLOW_PHONON_DEVICES=cpu`; run the ignored `phonon_live` integration test. No `--install` is needed.

## Checks and remaining work

Final locked Rust tests pass **983 tests, 27 ignored** across 45 targets; the library contributes 639 passes/16 ignored. All-target Clippy with warnings denied passes. Builder tests pass 8; Python tests pass 45. Formatting, scoped hashes and log digests are in `provenance.json`. Ignored tests are not passes. Frontend and refinement implementation hashes retain their earlier evidence; no additional LLM quality claim follows from these ASR checks.

Project CI will build preview installers for Linux, Windows and both Mac architectures plus the 18 dependency combinations. The dedicated workflow only uploads artifacts; it does not create a release, sign/notarize distribution installers or publish a runtime catalog. Physical clean-install/device tests are deferred by the user. Full CI results, accelerator operations, distribution review, broader faults, old-generation cleanup, the 15 allowed Rust advisory warnings and eight substantive writing-quality cases remain tracked in [implementation progress](../../implementation-progress.md). The objective remains incomplete.
