# Atomic activation and owned cancellation verification

> Historical source checkpoint. See [owned runtime policy revocation](network-policy-revocation-2026-10-23.md) for the subsequent failed-policy-write and runtime-lifecycle changes.

23 October 2026, current Linux worktree. The overall cross-platform release goal remains incomplete.

The speech runtime pointer, writing-runtime pointer and sidecar network-policy document now use one file writer in `src-tauri/src/platform/persistence.rs`. It creates a unique file beside the destination, writes and flushes it, closes it and replaces the destination. Errors and unwinding remove the owned temporary file. The previous file is never deleted as a fallback. The writing runtime’s existing bounded Windows sharing/antivirus retry was moved into this common boundary.

## Confirmed bugs and changes

- Speech Cancel could be accepted after pointer activation started and still be followed by success. Cancellation and activation now share the existing progress lock. Cancel winning first prevents promotion; activation winning first rejects cancellation until the verified commit finishes.
- Frontend Cancel could run before its setup attempt was known, or during the health check after that attempt had completed. The three setup controls now require an owned attempt ID and a cancellable phase. The new activation phase and terminal phases disable cancellation. Tests cover delayed ownership and delayed post-install verification.
- Reopening Model settings during activation could show a generic downloading phase. The legacy panel now reads progress from the status snapshot, and a terminal event wins over the older setting-up snapshot. It also offers offline cache repair without changing network preferences. The explicit download action remains separate.
- Complete imperative procedures were rendered like inventory fragments. The existing terminal-punctuation helper now supplies sentence endings only for safe prose endings in explicitly ordered imperative procedures. Command lines, technical/literal endings, existing punctuation and ordinary inventory fragments retain their treatment.
- A live low-temperature model candidate exposed a relationship-attachment error. The new finite-predicate/preposition regression is described in the [current transcription review](refinement-2026-10-23/hardening/README.md).

## Evidence and limits

| Check                                                | Result                     | Scope                                                                                                                                |
| ---------------------------------------------------- | -------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| Full locked Rust suite                               | 968 passed, 27 ignored     | Current Linux host, including 626 library tests; ignored tests are not passes                                                        |
| Full frontend check                                  | 56 suites / 353 tests pass | Includes setup ownership, activation snapshots and post-completion cancellation; component tests, not physical device tests          |
| All-target Clippy / Rust formatting / frontend build | Pass                       | Linux compilation; existing non-fatal CleanupPage and Vite size warnings remain                                                      |
| Writer failure injection                             | Pass                       | Partial-write and flush error returns; incumbent and temporary cleanup verified; not a physically full filesystem                    |
| Real filesystem replacement failures                 | Pass                       | Directory destination and parent-file failures; incumbent bytes and retry recovery verified                                          |
| Concurrent-reader contracts                          | Pass                       | Existing speech-policy and writing-pointer tests only observe complete documents                                                     |
| Windows locked-file recovery test                    | Added, not run here        | Holds a file without delete sharing, requires bounded failure, incumbent preservation and successful retry after unlock              |
| Real offline dependency pack                         | Pass                       | Same immutable 107,344,553-byte Linux Zipformer pack; install 16.476 seconds; full cancellation/corruption/repair test 35.75 seconds |
| Model downloads / app network journal in pack test   | 0 / 0                      | Model-free isolated app data; zero journal entries are not a packet capture                                                          |
| Static setup design scan                             | No findings                | Onboarding and both setup components; final visual/browser inspection is unavailable                                                 |

The model-free production dependency worker created and verified a fresh runtime, used the shared writer to activate it, preserved the incumbent through cancellation and corrupt/missing-cache failures, and repaired offline. The environment sets `APPDIR` and an invalid inherited `PYTHONHOME`; this reproduces launch-environment restoration, not an actual AppImage installation. Data and scoped source hashes are retained under `runtime-pack-linux-2026-10-23/atomic-activation-*.json`.

The writer guarantees atomic visibility under the tested conditions. It does not prove power-loss durability of all runtime files, cleanup after a hard crash, real disk-full behavior, Windows/macOS filesystem behavior, child-tree termination or signed/quarantined executables. A failed offline-policy write can still leave an already-running Python sidecar with an old policy; cross-process revocation remains a release blocker. Superseded-generation cleanup and native acceptance also remain open.

## Source boundaries changed

- `src-tauri/src/platform/persistence.rs` and `platform/mod.rs`: shared writer and fault tests.
- `src-tauri/src/asr/python_setup.rs`, `rewrite/runtime_inventory.rs` and `network_policy.rs`: writer reuse; speech activation/cancellation ordering.
- `src-tauri/src/formatting/normalizers.rs` and `rewrite/safety.rs`: procedure punctuation and targeted relationship validation.
- `src/services/speechSetup.ts`, `src/types/index.ts` and the three setup components: shared labels, owned cancellation and activation snapshots, with regression tests.
- `src-tauri/tests/fixtures/refinement_cases.json`: accept an existing safe exclusion paraphrase in the coarse check.

This reduces duplicate production write/replace code. The whole worktree still has net source growth from the added behavior and tests; `source-size-2026-10-23.json` reports the selected-language counts against the initial dirty-tree snapshot. No overall codebase shrink is claimed.
