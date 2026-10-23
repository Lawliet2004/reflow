# Failed policy saves, owned runtime revocation and startup recovery

> The subsequent [paragraph-preservation checkpoint](refinement-2026-10-23/paragraph-preservation/README.md) holds newer whole-worktree checks and refinement measurements. The scoped ASR/runtime evidence below remains historical and applicable to its unchanged sources.

23 October 2026, Linux host. The overall Windows/macOS/Linux reliability and transcription goal remains incomplete. This pass changes the ASR lifecycle; the measured refinement prompt, validator, normalization, context and decoding sources remain unchanged.

## Confirmed problems and resulting behavior

1. A failed network-policy write used a queued `unload_model_blocking` request. A busy ASR actor could hold it behind a stalled pipe/read, leaving the Python process running against its old online policy. The actor now retains a direct policy signal to the process it owns. Persistence and signal updates are serialized; a failed save revokes startup admission and attempts bounded termination without entering the actor queue. Both failed offline and failed online policy saves use this path, including startup and settings rollback.
2. A runtime swap could otherwise lose the revocation handle, or create a new Python process after failure. The old owned process must retire before its signal is replaced, and a replacement inherits the current policy availability. Startup rechecks admission while owning the child slot and after spawn. Retirement errors propagate; an unconfirmed live handle is retained and cannot be overwritten by a replacement process. The existing runtime-removal command aborts before deleting directories when shutdown returns an error.
3. A late status probe could return after failure and repair, restoring an old loaded state. The existing cached ASR status now carries an internal generation. A probe from an older generation cannot commit or promote readiness. Retirement also clears the engine's internal loaded/loading/downloading flags, so a fresh probe cannot reuse a stale ready state after its transport is gone. Status reads remain independent of the actor/long Python operation. Saving a repaired policy alone does not advertise ASR readiness.
4. Repair could otherwise require a manual model reload. A successful save reports whether an interrupted sidecar needs recovery. The settings path uses the existing deferred reload scheduler, which checks the latest selected model is already installed and waits for session admission. It does not download weights, change the selected model, or reload an unaffected native engine on an ordinary save.
5. Repeated initialization could replace a live child. Initialization is now idempotent for a live transport, and verified retirement precedes replacement. Intentional policy interruptions do not count as runtime crashes. The shared unavailable-status constructor keeps failure snapshots consistent.
6. An adversarial AppImage test exposed launcher restoration reintroducing an invalid original `PYTHONHOME` after the sidecar cleared Python overrides. Restoring the host environment now precedes clearing those overrides. The exact lifecycle fixture failed before the change and passed afterward. The full suite also runs an isolated, bounded child-test invocation with invalid inherited/original Python homes, so ordinary CI exercises this launch condition without mutating other tests' environment.

The existing Rust download gate still closes after failed persistence. Settings retain their saved preference and emit the existing error event; the stopped/blocked Python runtime is exposed as unavailable. If termination cannot be confirmed, the returned error includes that failure rather than claiming the process stopped. Local engines without this outbound sidecar retain their behavior.

## Controlled regression evidence

| Scenario                                  | What the test proves                                                                                                                                                                                                                         |
| ----------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Busy actor and readable old online policy | A real Python fixture holds a loopback connection while its command reader is stalled; a real invalid-parent persistence failure is delivered through the same result handler, closes the owned connection and releases the blocked ASR call |
| Unrelated process                         | A separate Python fixture remains responsive after revocation; no process-name/PID scan is used                                                                                                                                              |
| Failure before startup and after swap     | Neither old nor replacement runtime can start until a successful policy repair                                                                                                                                                               |
| Recovery/readiness                        | Repair reports the one-time reload need, leaves cached readiness false, and a new successful probe/start can establish readiness; an ordinary save does not request another reload                                                           |
| Late status after failure plus repair     | A gated older probe cannot overwrite the newer unloaded snapshot; a fresh probe can promote readiness                                                                                                                                        |
| Busy child owner / failed retirement      | The bounded stop returns a deadline error, unload reports failure, the live process handle stays owned, restart remains denied and a later stop can reap it                                                                                  |
| Native/local engine                       | A policy-file failure does not unload an engine without the outbound-sidecar signal                                                                                                                                                          |
| AppImage override order                   | Before: lifecycle failed after 4.59 seconds; after: the same fixture passed in 0.18 seconds with invalid inherited and original Python homes                                                                                                 |

The loopback fixture uses Python stdlib and synthetic protocol replies; it loads no ASR weights and performs no model download. The failed-write fixture intentionally supplies the persistence result from a real invalid-parent filesystem error without mutating the global network gate of concurrently running tests. The separate existing failed-persistence test proves that gate stays closed. These are compositional backend contracts, not a full Tauri settings/packaged-desktop fault journey or an actual ASR decode.

The stop path has a one-second ownership-lock/reap budget. The controlled test asserts the signal returns within two seconds. That is not a guarantee against arbitrary kernel/filesystem stalls or a substitute for native process-tree testing.

## Changed files and reuse

- `src-tauri/src/asr/engine.rs`: policy signal contract and common unavailable status.
- `src-tauri/src/asr/actor.rs`: serialized policy application, recoverability flag, checked engine replacement and generation-aware cached status.
- `src-tauri/src/asr/sidecar.rs`: shared owned child handle, bounded retirement, blocked/repaired startup, idempotence and launcher ordering.
- `src-tauri/src/asr/actor_policy_tests.rs`: model-free ownership, stale-policy connection and recovery fixtures.
- `sidecar_transport_tests.rs` and `sidecar_load_deadline_tests.rs`: existing fixtures use the shared child owner and check cleanup results.
- `src-tauri/src/context.rs` and `src-tauri/src/commands/mod.rs`: startup, normal settings and rollback use the same policy boundary; repair reuses the existing reload scheduler.

No dependency, model, inference server, frontend setting or Tauri IPC field was added. The existing transfer, atomic writer, actor and scheduler remain responsible for their current operations; no general orchestration framework was introduced.

## Verification and remaining work

Final locked full Rust results: **979 passed, 27 ignored**; library **636 passed/16 ignored**. All-target Clippy with `-D warnings` and Cargo formatting pass. Ignored tests are not passes. The final separate cached dependency-pack test also passes: fresh installation **16.589 seconds**, full cancellation/corruption/missing-cache/offline-repair run **36.13 seconds**, zero model downloads/network-journal entries, with invalid inherited and original Python homes. These are local observations, not a performance guarantee.

Scoped source hashes, check-log hashes and the separate pack result are retained in the adjacent provenance and [active tracker](../implementation-progress.md). The [current 200-trial refinement evidence](refinement-2026-10-23/object-retention/README.md) remains applicable to its unchanged pipeline; no extra model benchmark is claimed for this ASR-only change. The unchanged frontend retains the earlier 371-test/build evidence.

Actual Windows/macOS child behavior, descendants/Windows job containment, signed/quarantined executables, full packaged policy-failure/recovery and physical microphone/shortcut/insertion journeys remain unverified. Simulated `APPDIR` is not an installed AppImage. Real disk-full/power-loss/crash durability, generation cleanup, outstanding dependency advisories and eight writing-quality gaps remain open. Final browser inspection remains disabled in Work settings; no bypass was used. No release was published.
