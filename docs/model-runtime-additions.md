# Requested local model additions

Metadata verified 2026-10-24 using Hugging Face JSON metadata/model cards and pinned runtime source. No model weights were downloaded and no packages or runtimes were installed during implementation. New integrations are model-free tested; transcription quality, latency and memory have not been benchmarked on these weights.

## Writing and summaries

All files use Q4_K_M. Text-only inference needs no vision projector. The existing managed llama.cpp release is `b11146`; its source supports `qwen35`, `llama`, and `gemma4`. MiniCPM5-2B's upstream config specifies `LlamaForCausalLM`, not a custom MiniCPM architecture. Existing request generation passes `min_p: 0.0`, as required by MiniCPM's upstream usage guidance, and launches with Jinja templates.

| Reflow ID   | Repository                  | File                       |      Bytes | Revision                                 | SHA-256                                                          |
| ----------- | --------------------------- | -------------------------- | ---------: | ---------------------------------------- | ---------------------------------------------------------------- |
| qwen3.5-4b  | unsloth/Qwen3.5-4B-GGUF     | Qwen3.5-4B-Q4_K_M.gguf     | 2740937888 | e87f176479d0855a907a41277aca2f8ee7a09523 | 00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4 |
| minicpm5-2b | openbmb/MiniCPM5-2B-GGUF    | MiniCPM5-2B-Q4_K_M.gguf    | 1561318368 | 2079a22f3beaa4e306449978533478fe0522f4b3 | ec2d5801640099e97d8d7e8003ad4d81f336e757811f03a26173dddf386602fd |
| gemma4-e4b  | unsloth/gemma-4-E4B-it-GGUF | gemma-4-E4B-it-Q4_K_M.gguf | 4977171584 | bfc15c382204943c3a8fff0c750b94ae2364d7a3 | 85a896a047553e842f25297ee5b031d64ff30147d9c4af17b1e4b394cd1fab87 |

Sources: [Qwen quantization](https://huggingface.co/unsloth/Qwen3.5-4B-GGUF), [MiniCPM official GGUF](https://huggingface.co/openbmb/MiniCPM5-2B-GGUF), [MiniCPM config](https://huggingface.co/openbmb/MiniCPM5-2B/blob/main/config.json), [Gemma quantization](https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF), [runtime architectures at b11146](https://github.com/ggml-org/llama.cpp/blob/b11146/src/llama-arch.cpp).

## Omnilingual speech

Reflow selects the smallest published INT8 variant: `omnilingual-300m-int8`, from [Edison2ST's requested v2 repository](https://huggingface.co/Edison2ST/sherpa-onnx-omnilingual-asr-1600-languages-ctc-v2), revision `5b5fae1ca6777861e13c4ac9e80b5a10d700ab18`. Archive `sherpa-onnx-omnilingual-asr-1600-languages-300M-ctc-v2-int8-2026-02-05.tar.bz2` is 247417872 bytes with SHA-256 `b72bef9be75862684098e722d79fefecf9f10fefd3a0b2950738977b4c6b4147`.

The existing sherpa-onnx 1.13.4 lightweight dependency set exposes [`OfflineRecognizer.from_omnilingual_asr_ctc`](https://github.com/k2-fsa/sherpa-onnx/blob/v1.13.4/sherpa-onnx/python/sherpa_onnx/offline_recognizer.py). Reflow runs this model on CPU with INT8 weights and greedy decoding. It buffers audio and transcribes on stop; it has no online partial recognition. The adapter automatically recognizes language, without a forced-language input. Upstream advertises 1600 languages; Reflow makes no corpus-validation claim.

Upstream distributes only archives. Reflow pins and verifies the archive as the integrity boundary, copies exactly one regular `model.int8.onnx` and `tokens.txt` into its own model directory, and never uses arbitrary archive extraction. Each load verifies the archive and refreshes those files; links and duplicate members are rejected. Download setup reserves 1GB for the compressed archive, expanded files and atomic copies. CPU loading requires 3GB available RAM. These are conservative admission estimates, not measurements.

## Additional speech model setup (2026-10-24 follow-up)

- [moondream/parakeet-ultra](https://huggingface.co/moondream/parakeet-ultra), revision `73175eb7aeb0d82f1e2a6b53b3aabc10a90bcd0b`: now has **Download & use** with automatic setup using the existing Phonon dependency profile. Although upstream advertises Photon, its checkpoint uses standard Transformers `ParakeetForTDT` tensor names and shapes. Reflow loads those tensors locally, permits only the optional `vad_head.*` extras, uses the Fermion mel frontend with a nonperiodic Hann window, and decodes with a bounded TDT loop that separates token and duration logits. CPU uses FP32; CUDA uses BF16. The tokenizer, configuration, model card and weights are pinned and verified. This is an experimental Transformers integration; upstream Photon benchmark numbers do not apply.
- [ManiKumarAdapala/indic-transcribe-core-8bit](https://huggingface.co/ManiKumarAdapala/indic-transcribe-core-8bit), revision `2765ed1b553ea331af15cf8fa3fd25eda7b993a6`: now has **Download & use**, an isolated `indic` dependency profile, hashed Linux/Windows x64 CUDA 13 locks, a runtime-pack builder profile, and a local custom-code adapter. Setup verifies a real bitsandbytes GPU operation before transferring weights. All downloaded Python, tokenizer, configuration, license and weight files are pinned; all hashes are checked again before importing any model code. A private package enables upstream relative imports without `sys.path` mutation or `trust_remote_code=True`. The frontend consumes Reflow's mono 16 kHz arrays using the checkpoint's exact mel/window buffers, avoiding a second audio decoder or mismatched torchaudio binary. Explicit Indic language choices use a separate catalogue and do not expand Qwen's prompt tags. CPU requests are refused with a GPU requirement; neither adapter substitutes Qwen.

Both use the existing runtime-first setup coordinator and download progress/recovery flow. Model sizes are approximately 1.26 GB and 1.57 GB, with dependency downloads additional when needed. Full-size local transcription, language quality, latency and memory remain unverified; see [follow-up verification](verification/additional-asr-2026-10-24/README.md). Release installers require newly built runtime-pack assets/catalogs; this change does not publish them.

## Verification in this checkout

- Production frontend build passed after the final layout change.
- Full frontend suite: 398 passed. The final Models/Performance ordering correction was followed by 27 passing focused UI tests.
- Full Rust library suite: 685 passed, 18 environment/download-dependent tests ignored. A subsequently added Omnilingual runtime-selection regression passed in the final 15-test profile suite.
- Python model-free suite: 50 passed using the existing `/home/papan/.local/share/reflow/python-runtime/bin/python`. Downloader messages in these tests come from mocked temporary-directory fixtures; no real weights were downloaded. The system Python lacks NumPy, so it is not the test/runtime interpreter.
- Existing installed CPU `llama-server --version` returned build 11146, commit `7fe450e19`. Source architecture support was checked at the matching tag. No new-model inference was performed.
- Changed frontend files pass Prettier checks; Rust formatting and `git diff --check` pass. ESLint reports no errors and one existing effect-state warning in CleanupPage. Strict Clippy remains blocked by existing `too_many_arguments` in `audio/capture.rs:710` and `needless_return` in `meetings/inhibit.rs:10`; neither file was changed. Vite reports a main-bundle size warning.
- Browser layout inspected at 1100×800 and 640×640. Model navigation, task filters, disclosures, offline download blocking, and independent summary selection have automated coverage. The independent finish review's preset-ordering finding was fixed and scored resolved. Screenshots are under `.impeccable/review/`.

Browser preview uses fallback hardware/model state, not the machine's native inventory. The original checks above predate the additional adapters. They establish integration contracts, not real recognition quality, throughput or memory measurements for the new full-size weights.

## Runtime-first writing setup

A writing-model download now owns one asynchronous attempt spanning `checking_runtime`, `installing_runtime` (when required), another `checking_runtime`, then the model transfer phases. The shared llama.cpp runtime is selected from the refinement device preference. Its existing pinned installer verifies the archive, stages an immutable generation, smoke-tests it and activates it before the model downloader can fetch weights. A verified matching installation is reused.

Concurrent same-variant requests join the active runtime setup; conflicting variants are refused. Cancelling the model also cancels runtime setup when that model owns it. Cancelling a waiter leaves another owner's runtime setup intact. Runtime activation is atomic and a verified runtime may remain installed after cancellation; no model transfer begins after that cancellation. Runtime byte progress retains the `runtime:download-progress` event, while the model attempt reports its prerequisite phases through `intelligence:download-progress`.

Speech setup uses the same prerequisite ordering. Python speech setup is coordinated outside the Settings component, so closing Settings does not drop the pending model request. It waits for package setup and verification, then rechecks speech settings and offline mode before calling the native model installer. Native ASR prepares the shared llama.cpp runtime on a blocking worker before taking the model-operation lock, and rechecks the complete speech settings snapshot before model transfer. Sidebar progress and failure recovery remain accessible throughout setup. Custom interpreters/binaries are verified through the existing override path; new inference engines still require dedicated adapters and package manifests.

Follow-up validation: Rust library suite 688 passed / 18 ignored; frontend suite 405 passed with one obsolete button-label assertion, corrected and all 29 affected tests then passed; sidebar tests 5 passed after copy refinement. Production build, TypeScript, focused ESLint, Rust formatting and whitespace checks passed. Model-free setup tests verify deduplication, runtime failure blocking and changed-settings admission. No runtime packages or model weights were installed for these checks.
The final independent review caught an old writing failure masking a live runtime retry in the sidebar. Active retry progress now takes precedence; all six sidebar tests pass, including this regression.
