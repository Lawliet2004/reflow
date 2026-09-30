# Backend and model runtime verification

Work date: 2026-09-30. Branch: `codex/backend-model-runtime`. Models remain Qwen3-ASR 0.6B / 1.7B and Qwen3.5 0.8B / 2B Q4_K_M. No dependencies were added to the application, no commits were amended, and nothing was pushed.

## Changes and commits

| Phase | Commit                                                             | Result                                                                                                                                                                                                                   |
| ----- | ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 1     | `bc333f8`                                                          | Rewrite repeat penalty 1.0; correct 2B name; remove all complete think blocks and unfinished tails; explicit Python model metadata; 30-second GPU startup budget and visible CPU fallback warning.                       |
| 2     | `0dd6cd0`                                                          | Real upstream weight pins, checksum-gated pinned downloads, updated llama.cpp archives and smoke checks.                                                                                                                 |
| 3     | `bde5df6`                                                          | Silence-aligned Python segmentation, cancellation between segments, response token-cap warnings, and a timeout ceiling that covers the admitted one-hour recording.                                                      |
| 4     | `5eb09dc`                                                          | Experimental native ASR setting, official decoder/projector pairs, shared server lifecycle, audio chat protocol, dictionary echo guard, eight-second native segments, selection/UI integration and real-audio benchmark. |
| 5     | Commit containing this report; its hash is in the delivery message | Final review fixed Python cancellation through the actor; README corrected, exact Python requirements added, frontend build added to its CI job, and numpy pinned for CI selftests.                                      |

The binary-audio unpacker was retained: production Rust already sends bounded PCM16 binary frames, and Python already reads them. The brief's description of base64 transport and dead code was stale. Phase 3.3 therefore needed no transport change.

Final review found that the actor blocked Python cancel requests behind `stop_stream`, despite the sidecar reader supporting cancellation during decoding. Phase 5 changes the cancellation hook to a session-bound callback. Python shares its stdin writer under a mutex, sends an out-of-band cancel with reserved request ID 0, and continues matching normal replies by nonzero IDs. Native retains its atomic cancellation flag through the same hook. The new regression fixture verifies binary PCM arrives intact, cancellation reaches a blocked decoder, out-of-order acknowledgements are ignored, and the next session still works.

The original sidecar timeout ceiling was 45 minutes even though one hour of audio could be admitted at an estimated worst-case 3× real-time factor. It is now three hours, covering that calculation; the existing duration-scaled budget otherwise remains intact.

Two existing test assumptions were corrected rather than relaxed. The runtime-flavor unit test assumed the user's installed marker was absent; it now checks unknown, matching and mismatched flavors with explicit inputs. Manifest language lists mean _validated_ languages: native entries have none until a real corpus comparison succeeds, and the consistency test asserts that distinction. The frontend settings expectation now includes the required Python runtime default. The overlay test module was moved after production items to satisfy the existing Clippy lint without changing behavior.

## Runtime benchmark and default

**Python remains the default. Native is experimental and has not passed the accuracy gate.**

The isolated Phase 4 run used Windows, Python 3.11.9, an NVIDIA RTX 2050 with 4 GiB VRAM, CUDA torch 2.13.0+cu130, and INT8 for both Python model sizes. The input was a single 10.54775-second, 24-word English Windows SAPI sample, with surrounding silence. This is a smoke measurement, not a representative accuracy corpus. Latency is final decode time, excluding model load. VRAM is the driver-reported free-memory delta after inference, not a measured peak. Raw output is in [asr-runtime-benchmark.json](asr-runtime-benchmark.json).

| Model | Runtime | Language | WER (%) | Decode latency (s) | Resident VRAM delta (MiB) | Status                                        |
| ----- | ------- | -------- | ------: | -----------------: | ------------------------: | --------------------------------------------- |
| 0.6b  | python  | en       |    0.00 |              6.728 |                      1362 | Measured                                      |
| 0.6b  | python  | hi       |       — |                  — |                         — | No hi audio references in the corpus          |
| 0.6b  | native  | en       |       — |                  — |                         — | Model files for native-0.6b are not installed |
| 0.6b  | native  | hi       |       — |                  — |                         — | No hi audio references in the corpus          |
| 1.7b  | python  | en       |    0.00 |              6.542 |                      2666 | Measured                                      |
| 1.7b  | python  | hi       |       — |                  — |                         — | No hi audio references in the corpus          |
| 1.7b  | native  | en       |       — |                  — |                         — | Model files for native-1.7b are not installed |
| 1.7b  | native  | hi       |       — |                  — |                         — | No hi audio references in the corpus          |

The gate requires measured Python and native WER for both model sizes in both English and Hindi, with an absolute difference no greater than 0.5 percentage points for each pair. Missing measurements fail it. The benchmark never edits user settings automatically.

The first trial used a misordered reference phrase and overlapped GPU integration tests; its 66.67% WER and memory figures were discarded. The table above is the corrected, isolated run.

To run the real benchmark from the repository root:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --bin reflow -- --benchmark path/to/corpus.json
```

The corpus is a JSON array of objects with `language` (`en` or `hi`), `audio` (WAV path relative to the corpus file) and `reference` (actual spoken text). Audio must be 16 kHz mono PCM16. Install both sizes for both runtimes first. The Unicode WER normalizer preserves Hindi combining marks; punctuation and case are normalized.

## Verification commands and results

All listed cargo commands ran from the repository root with `--manifest-path src-tauri/Cargo.toml`; `-j 1` limits compiler memory use on this Windows machine. Frontend commands ran from the repository root.

| Phase | Command or check                                                                                                                | Result                                                                                                                                                                                                                                                            |
| ----- | ------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 1     | `cargo test --manifest-path src-tauri/Cargo.toml`                                                                               | Initial attempt failed from Windows paging/resource pressure while linking; subsequent default-parallel attempt could not find the reqwest rlib after the interrupted build.                                                                                      |
| 1     | `cargo clean --manifest-path src-tauri/Cargo.toml -p reflow`                                                                    | Initially blocked by the running development executable. The workspace Reflow process was stopped to release the lock; it was not relaunched.                                                                                                                     |
| 1     | Existing unit test executable, direct run                                                                                       | 228/229 passed; the installed-marker-dependent assertion failed and was corrected as explained above.                                                                                                                                                             |
| 1     | `cargo test --manifest-path src-tauri/Cargo.toml -j 1`                                                                          | Passed: 229 unit tests plus integration/live tests.                                                                                                                                                                                                               |
| 1     | `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -j 1 -- -D warnings`                                           | Passed after moving the overlay test module and replacing the machine-dependent flavor assertion.                                                                                                                                                                 |
| 1     | `python model-runtime/qwen3_asr_runtime.py --selftest`                                                                          | Passed: 63 checks.                                                                                                                                                                                                                                                |
| 2     | `cargo test --manifest-path src-tauri/Cargo.toml -j 1`                                                                          | Passed: 357 tests across all targets, including 229 unit tests.                                                                                                                                                                                                   |
| 2     | `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -j 1 -- -D warnings`                                           | Passed.                                                                                                                                                                                                                                                           |
| 2     | `python model-runtime/qwen3_asr_runtime.py --selftest`                                                                          | Passed: 66 checks, including valid weight, corruption/deletion and missing-file checks.                                                                                                                                                                           |
| 2     | `cargo run --manifest-path src-tauri/Cargo.toml --bin runtime_install_smoke -j 1`                                               | Passed: every smoke check.                                                                                                                                                                                                                                        |
| 2     | Download Windows Vulkan b11146 archive, SHA-256 verify, extract, run `llama-server.exe --help`                                  | Passed. Confirmed `--jinja`, `--reasoning off`, `--ctx-size`, `--parallel`, `--n-gpu-layers`, `--main-gpu`, `--mmproj` and `--no-mmproj-offload`. The last flag disables projector GPU offload when native CPU is selected.                                       |
| 2     | Fresh 0.6B HF snapshot, HTTP retry and parallel range download scripts                                                          | Stopped incomplete because sustained Hugging Face download throughput was too low. No fresh-download verification pass is claimed.                                                                                                                                |
| 2     | Hash full installed 0.6B file; copy it, flip one byte, verify again                                                             | Passed. Installed file matched the API digest; corrupted copy was rejected and deleted by the production verifier. Original weights were preserved.                                                                                                               |
| 3     | `python model-runtime/qwen3_asr_runtime.py --selftest`                                                                          | Passed: 74 checks, including silence cuts, continuous loud audio, maximum duration, complete coverage, concatenation, token warnings and cancellation.                                                                                                            |
| 3     | `cargo test --manifest-path src-tauri/Cargo.toml -j 1`                                                                          | Passed: 357 tests.                                                                                                                                                                                                                                                |
| 3     | `python .codex-runtime-evidence/test_long_dictation.py`                                                                         | Passed with real 0.6B CUDA INT8 inference on 305.88475 seconds of speech: all 696 words and 29 repeated phrases returned in 190.284 seconds; no warning. Reference and transcript word sequences match after case/punctuation normalization.                      |
| 4     | `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -j 1 -- -D warnings`                                           | Initial attempt caught use of LazyLock beyond the declared Rust minimum; changed to OnceLock. Final runs passed.                                                                                                                                                  |
| 4     | `cargo test --manifest-path src-tauri/Cargo.toml -j 1` (two full runs)                                                          | Passed. Final full run: 367 tests, including 237 unit tests, real Python status convergence, and real refinement server tests.                                                                                                                                    |
| 4     | `cargo test --manifest-path src-tauri/Cargo.toml --lib -j 1`                                                                    | Passed: 237 tests, including actual actor cancellation of a blocked native HTTP decode and settings migration/persistence.                                                                                                                                        |
| 4     | `npm run test`                                                                                                                  | First run: one expected-object failure due to the new runtime field; final rerun: all 66 tests in 17 files passed.                                                                                                                                                |
| 4     | `npm run build` (initial and final runs)                                                                                        | Passed TypeScript compilation and Vite production build.                                                                                                                                                                                                          |
| 4     | `npm run lint` (initial and final runs)                                                                                         | Exit 0; zero errors, five pre-existing React hook warnings.                                                                                                                                                                                                       |
| 4     | `cargo fmt --manifest-path src-tauri/Cargo.toml --check` and `git diff --check`                                                 | Passed.                                                                                                                                                                                                                                                           |
| 4     | `cargo run --manifest-path src-tauri/Cargo.toml --bin reflow -j 1 -- --benchmark .codex-runtime-evidence/benchmark-corpus.json` | Two real runs completed. Final isolated measurements are in the table; missing native/Hindi rows leave the gate false.                                                                                                                                            |
| 4     | `python .codex-runtime-evidence/native-download-probe.py`                                                                       | 4,718,592 bytes in 60.407 seconds from the pinned 0.6B decoder URL, out of 804,749,248 bytes. No model was installed. Native live dictation could not be run.                                                                                                     |
| 5     | `python model-runtime/qwen3_asr_runtime.py --selftest`                                                                          | Passed: all 74 checks after documentation/requirements changes.                                                                                                                                                                                                   |
| 5     | Query installed package metadata and exact-version PyPI JSON endpoints                                                          | All five versions match the tested environment and are published. Official CUDA wheel index contains the tested Windows Python 3.11 torch wheel.                                                                                                                  |
| 5     | `npx prettier --write` on changed TS, README, CI and JSON/report artifacts                                                      | Changed artifacts formatted; no semantic test change beyond adding the runtime default.                                                                                                                                                                           |
| 5     | `npm run format:check`                                                                                                          | Failed on 82 pre-existing Windows checkout files with CRLF conversion. Checking the original Git sources of all 82 with the same Prettier configuration passed; these are checkout line endings, not new formatting errors. Unrelated files were not reformatted. |

Additional final checks: Prettier passed for all changed artifacts; a source/ledger consistency script confirmed every model and archive pin, all installed requirement versions and all six required CI commands.

Final Phase 5 verification after the cancellation repair:

- `cargo test --manifest-path src-tauri/Cargo.toml -j 1`: all 368 tests passed, including 238 unit tests and the live integration checks.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib -j 1`: all 238 unit tests passed again on the final source.
- `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -j 1 -- -D warnings`: passed.
- `cargo fmt --manifest-path src-tauri/Cargo.toml --check` and `git diff --check`: passed.
- `npx prettier --check` on all changed frontend/document/CI artifacts: passed.
- The five-minute reference and hypothesis matched word-for-word after case/punctuation normalization (696 words each).

The full Rust suite's real-model tests ran on this machine because the Python and refinement weights were installed. They may skip their live portion on CI without weights. No hosted GitHub Actions run was triggered or observed here.

## Unfinished verification and limits

- A **new** complete 0.6B HF download did not finish despite snapshot, plain HTTP and parallel range attempts. Installed-file verification and full-file one-byte corruption rejection passed, but they do not replace the fresh-download requirement.
- Native decoder/projector pairs were not installed. The measured HF throughput would require hours for the 1.02 GB and 2.52 GB pairs. Native chat serialization, segmentation, dictionary handling, cancellation and warning behavior are tested against a local HTTP fixture; recognition with actual native weights is unverified.
- No Hindi recording with reference text was provided, and this system only has English SAPI voices. Hindi WER and all native WER/latency/VRAM remain unmeasured. No comparison or default promotion is claimed.
- AMD, Intel and macOS acceleration were not tested on this Windows/NVIDIA machine. Shared platform launch paths are reused, but native acceleration and memory estimates still need real hardware validation.
- Live Python verification used generated speech fed through the runtime/sidecar, including the five-minute test and both benchmark sizes. An interactive microphone-to-paste session was not performed.
- The pre-existing `src-tauri/Cargo.toml` worktree change was left untouched and excluded from every commit. Large downloads, WAVs and raw command logs remain under ignored `.codex-runtime-evidence/` for local inspection.

## Model pins and API provenance

These were read from the listed Hugging Face API URLs: repository commit from `sha`, weight SHA-256 from `siblings[].lfs.sha256`, and byte size from `siblings[].lfs.size`. Each ASR HF repo currently has one safetensors file. Native models additionally require the listed projector. The machine-readable ledger is [runtime-pins.json](runtime-pins.json).

### Qwen/Qwen3-ASR-0.6B-hf

API: [Qwen/Qwen3-ASR-0.6B-hf metadata](https://huggingface.co/api/models/Qwen/Qwen3-ASR-0.6B-hf?blobs=true). Revision: `7f1569a48a89f3e3f4dc3a5c9d28bddd903bc76c`.

| File                | SHA-256                                                            |      Bytes |
| ------------------- | ------------------------------------------------------------------ | ---------: |
| `model.safetensors` | `d3f212dd20abecd315d830bc54ae3865e56ebfc3276484e57b771288ba27fd35` | 1564928088 |

### Qwen/Qwen3-ASR-1.7B-hf

API: [Qwen/Qwen3-ASR-1.7B-hf metadata](https://huggingface.co/api/models/Qwen/Qwen3-ASR-1.7B-hf?blobs=true). Revision: `bcd2b5b7f32b480ab5790554cfa8347f246a14f3`.

| File                | SHA-256                                                            |      Bytes |
| ------------------- | ------------------------------------------------------------------ | ---------: |
| `model.safetensors` | `2db53c7d81bd9b8cbc6a074e89be2c968a0d373fb4ee68bb1b1e14f7042dfee1` | 4076193080 |

### unsloth/Qwen3.5-0.8B-GGUF

API: [unsloth/Qwen3.5-0.8B-GGUF metadata](https://huggingface.co/api/models/unsloth/Qwen3.5-0.8B-GGUF?blobs=true). Revision: `6ab461498e2023f6e3c1baea90a8f0fe38ab64d0`.

| File                       | SHA-256                                                            |     Bytes |
| -------------------------- | ------------------------------------------------------------------ | --------: |
| `Qwen3.5-0.8B-Q4_K_M.gguf` | `bd258782e35f7f458f8aced1adc053e6e92e89bc735ba3be89d38a06121dc517` | 532517120 |

### unsloth/Qwen3.5-2B-GGUF

API: [unsloth/Qwen3.5-2B-GGUF metadata](https://huggingface.co/api/models/unsloth/Qwen3.5-2B-GGUF?blobs=true). Revision: `f6d5376be1edb4d416d56da11e5397a961aca8ae`.

| File                     | SHA-256                                                            |      Bytes |
| ------------------------ | ------------------------------------------------------------------ | ---------: |
| `Qwen3.5-2B-Q4_K_M.gguf` | `aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223` | 1280835840 |

### ggml-org/Qwen3-ASR-0.6B-GGUF

API: [ggml-org/Qwen3-ASR-0.6B-GGUF metadata](https://huggingface.co/api/models/ggml-org/Qwen3-ASR-0.6B-GGUF?blobs=true). Revision: `928ab958557df9aa2ef1c93e0e83c7ad0933fae2`.

| File                              | SHA-256                                                            |     Bytes |
| --------------------------------- | ------------------------------------------------------------------ | --------: |
| `Qwen3-ASR-0.6B-Q8_0.gguf`        | `bca259818b50ca7c4c05e9bdb35a5dc04fa039653a6d6f3f0f331f96f6aa1971` | 804749248 |
| `mmproj-Qwen3-ASR-0.6B-Q8_0.gguf` | `41a342b5e4c514e968cb756de6cd1b7be39eff43c44c57a2ef5fc6522e36603d` | 214392480 |

### ggml-org/Qwen3-ASR-1.7B-GGUF

API: [ggml-org/Qwen3-ASR-1.7B-GGUF metadata](https://huggingface.co/api/models/ggml-org/Qwen3-ASR-1.7B-GGUF?blobs=true). Revision: `36a678687ba7d07a74ca70ccb0e36902e005fb80`.

| File                              | SHA-256                                                            |      Bytes |
| --------------------------------- | ------------------------------------------------------------------ | ---------: |
| `Qwen3-ASR-1.7B-Q8_0.gguf`        | `58e22d0532d4eacaf034cfac17a6fed159f37c41390c710186783be439d1fc57` | 2165034944 |
| `mmproj-Qwen3-ASR-1.7B-Q8_0.gguf` | `46c1d533af3f354ceb37ce855dbceff7da7fa7cf1e6a523df3b13440bd164c0d` |  355709344 |

## llama.cpp archive pins and API provenance

Stable release: `v0.5.0` from [GitHub latest release API](https://api.github.com/repos/ggml-org/llama.cpp/releases/latest). Its [nightly tag asset](https://github.com/ggml-org/llama.cpp/releases/download/v0.5.0/nightly-tag.txt) contains `b11146`. Each archive below came from [the binary release API](https://api.github.com/repos/ggml-org/llama.cpp/releases/tags/b11146), using `assets[].name`, `browser_download_url`, `size` and `digest` (the SHA-256 prefix removed).

| Archive                                                                                                                                                   | SHA-256                                                            |    Bytes |
| --------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------ | -------: |
| [llama-b11146-bin-macos-arm64.tar.gz](https://github.com/ggml-org/llama.cpp/releases/download/b11146/llama-b11146-bin-macos-arm64.tar.gz)                 | `1ad3f9eff80edb9dbef4259ad564d1720612ef7eea48fa4afed0e54f5f3d5711` | 11189714 |
| [llama-b11146-bin-macos-x64.tar.gz](https://github.com/ggml-org/llama.cpp/releases/download/b11146/llama-b11146-bin-macos-x64.tar.gz)                     | `305f0e3a17d2c01eb205cd0a62128357f1ec3b55329cb084d94e5ec0115d7a3b` | 11237237 |
| [llama-b11146-bin-ubuntu-arm64.tar.gz](https://github.com/ggml-org/llama.cpp/releases/download/b11146/llama-b11146-bin-ubuntu-arm64.tar.gz)               | `4aeda6fe68831547e49b7fa87607383ca5352b3d72ca5f70d52ed265f58c131f` | 13598346 |
| [llama-b11146-bin-ubuntu-vulkan-arm64.tar.gz](https://github.com/ggml-org/llama.cpp/releases/download/b11146/llama-b11146-bin-ubuntu-vulkan-arm64.tar.gz) | `5dcebe3ecbcb43a1ed85e3284453f9edf54dcca833e1cb1f54b4022b753c1da5` | 24410274 |
| [llama-b11146-bin-ubuntu-vulkan-x64.tar.gz](https://github.com/ggml-org/llama.cpp/releases/download/b11146/llama-b11146-bin-ubuntu-vulkan-x64.tar.gz)     | `d3ce40fce7403cc93bcf5718fc46c6efb61ed9709f8e5d9f10c86bf0e30e8fb3` | 30598492 |
| [llama-b11146-bin-ubuntu-x64.tar.gz](https://github.com/ggml-org/llama.cpp/releases/download/b11146/llama-b11146-bin-ubuntu-x64.tar.gz)                   | `c150306eb16b5ab696f76a8bdf810c35fd98a24e82158742e6fa28f420ff8410` | 16998357 |
| [llama-b11146-bin-win-cpu-x64.zip](https://github.com/ggml-org/llama.cpp/releases/download/b11146/llama-b11146-bin-win-cpu-x64.zip)                       | `14cf1303ca9ac3abd94816850532f9f9a69ac66fbaca3776fc6f9061c2fac1d1` | 18560055 |
| [llama-b11146-bin-win-vulkan-x64.zip](https://github.com/ggml-org/llama.cpp/releases/download/b11146/llama-b11146-bin-win-vulkan-x64.zip)                 | `55a378aa095b466979d85075234f66d7655c7a7483222af0c006c0e55b4d7bd6` | 32127004 |

Native protocol references: [upstream Qwen3-ASR PR #19441](https://github.com/ggml-org/llama.cpp/pull/19441), [pinned server documentation](https://github.com/ggml-org/llama.cpp/blob/b11146/tools/server/README.md), and [pinned multimodal documentation](https://github.com/ggml-org/llama.cpp/blob/b11146/tools/mtmd/README.md). The protocol uses local `/v1/chat/completions`, WAV `input_audio`, and `generation_prompt` for forced-language ASR prefixes.

## Python requirement provenance

The exact pins in [requirements.txt](../model-runtime/requirements.txt) match installed package metadata and these published package APIs:

- `torch==2.13.0` — [torch 2.13.0 API](https://pypi.org/pypi/torch/2.13.0/json).
- `transformers==5.15.1` — [transformers 5.15.1 API](https://pypi.org/pypi/transformers/5.15.1/json).
- `torchao==0.18.0` — [torchao 0.18.0 API](https://pypi.org/pypi/torchao/0.18.0/json).
- `huggingface_hub==1.11.0` — [huggingface_hub 1.11.0 API](https://pypi.org/pypi/huggingface_hub/1.11.0/json).
- `numpy==2.4.4` — [numpy 2.4.4 API](https://pypi.org/pypi/numpy/2.4.4/json).

The tested CUDA wheel comes from the [official CUDA 13.0 torch index](https://download.pytorch.org/whl/cu130/torch/). The installed local version is `2.13.0+cu130`; the requirements pin its `2.13.0` release so the appropriate platform wheel can be installed separately. These pins require Python 3.11+ because numpy 2.4.4 requires it.
