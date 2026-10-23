# Linux Zipformer runtime-pack verification

Measured 23 October 2026 (Asia/Kolkata) on the current Kubuntu x64 development host. This is model-free dependency and recovery evidence, **not clean-installer or physical dictation acceptance**. Windows, macOS, other packs, microphone permissions, external insertion, signing/quarantine and actual accelerator execution remain unverified.

The pack was assembled from the checked-in x64 Zipformer lock and pinned CPython 3.12.15 archive. The build installed every wheel locally with socket connections denied, passed `pip check` and imported NumPy/sherpa-onnx. The production Rust installer was then exercised in a fresh, isolated app-data directory with offline mode enabled. It invoked the unpacked interpreter directly, created the environment at its final generation path and fetched no models. No system Python environment supplied its dependencies.

| Evidence                                  | Result                                                             |
| ----------------------------------------- | ------------------------------------------------------------------ |
| Pack                                      | `speech-runtime-x86_64-unknown-linux-gnu-zipformer-none.tar.gz`    |
| Archive bytes                             | 107,344,553                                                        |
| Declared extracted payload bytes          | 109,078,765 (before Python extraction/environment installation)    |
| SHA-256                                   | `a41e50ae4e674a58a0ffcf6af372def05144711795f46f35d8cc63a0702b5cbf` |
| Initial isolated offline install          | 19.613 seconds                                                     |
| Bounded-probe isolated offline install    | 20.057 seconds                                                     |
| Simulated AppImage / invalid PYTHONHOME   | 19.011 seconds; inherited override removed                         |
| Final policy-gate source, same simulation | 17.924 seconds                                                     |
| Shared writer / activation boundary       | 16.476 seconds; earlier shared-writer checkpoint                   |
| Setup recovery / progress revisions       | 17.215 seconds; invalid inherited PYTHONHOME cleared               |
| Owned runtime revocation / launcher order | 16.589 seconds; invalid inherited/original homes cleared           |
| Failed-save diagnostics / atomic state    | 16.728 seconds; offline admission/recovery retained                |
| Cancel at extraction                      | Cancelled; incumbent and cached pack preserved                     |
| Corrupt or missing cache while offline    | Rejected; no network fallback or promotion                         |
| Retry from verified cache                 | New generation installed offline; incumbent retained               |
| Actual interpreter/import checks          | Passed                                                             |
| Network journal entries / model downloads | 0 / 0                                                              |
| Complete final opt-in test                | Passed, 37.31 seconds including repair/fault cases                 |

Timing is a local observation under changing desktop/build load, not a setup-time guarantee. A zero-entry app journal is not a packet capture. The separate build probe denied sockets; the production path used the app's offline gate and local `--no-index` installation.

The AppImage-environment runs set `APPDIR` and an invalid original `PYTHONHOME` to reproduce launch-environment restoration. They do not run an AppImage installer.

The setup-recovery run also sets `PYTHONHOME` itself to an invalid value outside the simulated AppImage. Installation completed, but its initial extra test-only health subprocess inherited that value and failed. The assertion now uses the application's bounded Python probe, which clears Python overrides. The complete installation/cancellation/corrupt-cache/missing-cache/offline-repair test then passed. This fixes the acceptance harness; the production installer/probe already cleared the override.

The latest run sets both inherited and original Python homes to invalid values. A separate isolated sidecar-lifecycle regression found and fixed restoration order in actual ASR startup; this dependency-worker test alone would not have exercised that startup path. See [policy/runtime verification](../network-policy-revocation-2026-10-23.md).

The unmodified archive is retained in `/home/papan/.cache/reflow-runtime-pack-verification/`, outside the repository and release catalog. Adjacent JSON files preserve the descriptor, build check, license inventory summary, the offline runs and current installer source hashes. Full license and notice text remains inside the pack and installed environment; the summary is not a legal distribution approval.

The worktree was already dirty. The descriptor's base commit alone does not identify all implementation changes. The versioned provenance files identify the corresponding installer sources; [policy recovery provenance](../policy-recovery-message-provenance-2026-10-23.json) covers the latest worker run; `policy-revocation-provenance.json`, setup-recovery and atomic-activation files preserve earlier checkpoints. The original provenance explains the earlier pack build. This pack was not published or presented as another platform's artifact. Use the commands in [runtime-packs.md](../../runtime-packs.md) to reproduce the isolated opt-in test.

The subsequent [policy recovery diagnostic check](../policy-recovery-message-2026-10-23.md) retains the new raw result in `policy-recovery-offline-install.json`; older raw runs remain preserved.

The subsequent [Qwen CPU/readiness and storage checkpoint](../runtime-pack-qwen-linux-2026-10-23/README.md) also reruns this unchanged Zipformer archive against final installer sources: 17.177 seconds to install, 37.31 seconds overall. Its `zipformer-regression.json` and provenance hold the latest worker evidence; older raw runs remain historical.
