# Dictation quality and recovery pass

13 October 2026, Reflow 0.4.0. This pass addresses concrete refinement and recovery defects. **Wispr Flow parity is not established. Recognition accuracy and native recording-to-paste behavior need further qualification.**

## Changes

- Preserve ordinary “no, wait” requests instead of deleting their negation. Resolve explicit, same-class corrections with ASR punctuation, such as “Friday, I mean, Thursday.” Corrections do not cross dictated paragraph boundaries.
- Keep quoted filler words, repetitions, repair markers, code, and literal whitespace out of destructive cleanup. Retain punctuation connecting a quotation to surrounding prose. AI cleanup rejects changed quoted content; its prompt explicitly requests verbatim quotations and code. The rewrite tokenizer handles enclosing apostrophes while preserving contractions.
- Count expired generation waits toward the shared refinement circuit breaker. Three consecutive failures disable further requests temporarily; late responses cannot reset the breaker. Both dictation refinement and local tasks use the same wait handling.
- Clean up recording state after a streaming recognizer fails to start. A retry can start normally.
- Surface incremental Zipformer decoding failures instead of returning a successful, incomplete transcript. Clear stale partial text, stop decoding the failed session, and reset the error when the session closes. A subsequent healthy recording succeeds.

Existing worktree changes and saved preferences were retained. No commit, publication, system installation, or restart of the running AppImage was performed.

## Verification

| Check                                                                    | Result                              |
| ------------------------------------------------------------------------ | ----------------------------------- |
| Rust library tests                                                       | 522 passed, 10 opt-in tests ignored |
| Dictation polish quality                                                 | 47 passed                           |
| Writing structure quality                                                | 21 passed                           |
| Shortcut/session ownership                                               | 11 passed                           |
| Long-form handling                                                       | 5 passed                            |
| Task completion regressions                                              | 8 passed                            |
| Simulated 100-cycle dictation stress                                     | 1 passed; uses mock recognition     |
| Python runtime tests                                                     | 39 passed                           |
| Rust Clippy, all targets, warnings denied                                | Passed                              |
| Rust formatting / working-tree whitespace                                | Passed                              |
| TypeScript and Vite production build                                     | Passed                              |
| Final AppImage `--status`, isolated data directory, launched from `/tmp` | Passed; CLI startup only            |

The new tests exercise punctuated and quoted corrections, quoted hesitations and repetitions, contractions, a stalled generation with late replies, failed stream admission, a decode failure after earlier speech, and successful recovery.

The installed Zipformer CPU INT8 model was also exercised with the existing 6.625-second labelled English WAV from the [official Zipformer example](https://k2-fsa.github.io/sherpa/onnx/pretrained_models/online-transducer/zipformer-transducer-models.html), using one untimed warmup and 20 warm repetitions. [Raw observations](dictation-quality-results.json) record:

- Word error rate: 5.56%; character error rate: 3.45%. These match the prior fixture results, so no recognition accuracy improvement is claimed.
- Complete recognition median / p95: 172.41 / 185.87 ms. Final stream drain median / p95: 7.51 / 9.92 ms. These are inference measurements, not microphone-to-insert latency, and do not demonstrate a speedup.

Twenty repetitions of one recording are not twenty independent accuracy samples. Accent, microphone noise, Hindi/mixed-language speech, proper names, and technical vocabulary are not qualified by this corpus. No live model rewrite or visible native paste result is claimed.

## Setup that still limits quality

The saved configuration selects Zipformer 20M, `raw_verbatim`, and no refinement model. Basic deterministic cleanup runs, but AI grammar refinement is disabled. Qwen3-ASR 0.6B and Qwen3.5 0.8B weights were absent when this pass started.

The missing models were requested from the application's pinned revisions through its existing network guard and checksum verification. Downloads are incomplete and have repeatedly stalled; partial files remain resumable and are not treated as installed weights. The download process may continue independently. Saved model selection was not changed to an unqualified or incomplete model.

After the weights and llama-server runtime are installed, select a writing model and Natural cleanup in Reflow. Verify the real pipeline with representative recordings and visible insertion into a browser, editor, and Konsole. The existing running AppImage contains the previous build; quit it before starting an updated build, because Reflow is single-instance.

## Review builds

Updated installers and their SHA-256 files are available in `src-tauri/target/release/bundle/appimage/Reflow_0.4.0_amd64.AppImage` and `src-tauri/target/release/bundle/deb/Reflow_0.4.0_amd64.deb`. The AppImage has the project's Python-environment and native desktop launch hooks. Both packages' staged speech runtime matches the tested source.

- AppImage SHA-256: `4c91d064a7339b61ab3d353b446928db35b8886f450d687133c21bca0913663c`
- Debian SHA-256: `9018cb74f7eec03d477790bcc1ae3de38681b6b7330f2c77cfe9bba216c83dc2`

The bundler clears its output directory. The previous running AppImage was recovered from its still-open executable inode, copied to `src-tauri/target/release/previous-quality-build/Reflow_0.4.0_amd64.AppImage`, and verified byte-for-byte by SHA-256. The running process was not stopped.
