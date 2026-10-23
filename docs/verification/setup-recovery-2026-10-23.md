# Reopened speech setup and ordered progress

> Historical source checkpoint. [Owned runtime policy revocation](network-policy-revocation-2026-10-23.md) addresses the subsequent policy-persistence failure path.

23 October 2026, Linux worktree. Windows, macOS and Linux remain in scope; no release or native support claim is implied by this report.

## Confirmed races and changes

- Reopening the dependency screen during installation could leave it waiting without joining the backend job. The screen now observes the exact captured attempt. A failed, disappeared or replaced job cannot silently become a fresh installation. Passive checks never enable downloads or install missing dependencies.
- Missing completion events or a failed event subscription could leave progress stuck. The owned wait polls a new lightweight metadata command once per second, retries transient IPC failures, and detaches on completion/unmount. Full Python probes and directory scans are reserved for readiness checks rather than every polling tick. The older Model settings panel also recovers missed terminal events through this cheap snapshot.
- Delayed verification cleanup could release a newer operation or notify its parent that outdated dependencies were ready. Controller identity and component generations now guard progress, readiness callbacks and cleanup. Changing model/device/interpreter invalidates the previous observer without cancelling another surface's backend job.
- Delayed progress snapshots/events could move activation backward, revive an older attempt or re-enable Cancel. The existing backend snapshot now carries a revision that increases across attempts within the backend process. Frontend event/snapshot merges retain the newest native observation. Final backend phases are immutable, including against late updates from the same owner. Backend revisions are not a persisted cross-process job database.
- A delayed full-status reply could say no setup was running after a newer start event. The dependency screen now joins that newer observed attempt rather than displaying an install button while another surface is already installing.
- The older panel could briefly show Retry after acknowledgement but before the new attempt snapshot arrived. Its local pending state now remains visible until the fresh status read finishes, and cancellation still requires an owned attempt ID.
- Heavy status work captured its setup phase before Python probes and directory scans. The snapshot is now captured after that work; probes cannot leave the returned phase frozen at their starting state. This reduces an observed ordering hazard; status inspection does not grant an indefinite filesystem/readiness lease.

The backend ownership/activation lock, shared transfer, atomic pointer writer and final native dependency verification remain in use. The metadata command does not probe Python, inspect package versions or walk installation directories. No additional orchestration framework, dependency or model was added.

## Regression evidence

The focused suites cover reopening offline without granting downloads, completion before attachment, stored failure/disappearance without restart, subscription failure plus transient polling failure, replacement ownership, stale poll/event revisions, old verification resolve/reject after replacement, missing receipts after verification, delayed acknowledgement, legacy missed completion and late replies after unmount. Fake timers control IPC/event ordering; these are component/service contracts, not native installer evidence.

The backend setup ownership test verifies increasing revisions across attempts, capture of model/device, stale-owner rejection, cancellation versus activation, and immutable terminal revisions. Existing runtime-pack and transfer tests retain their fault coverage.

The additional real, isolated offline dependency-worker run installed the existing Linux Zipformer pack in 17.215 seconds and passed cancellation, corrupt/missing cache, offline repair and incumbent preservation in 36.82 seconds overall. It used simulated `APPDIR` plus an invalid inherited `PYTHONHOME`, with zero model downloads/network journal entries. Its first extra test-only health subprocess failed because it inherited that invalid environment after installation had succeeded. The assertion now uses the app's bounded Python probe. See [versioned pack evidence](runtime-pack-linux-2026-10-23/README.md). This remains model-free worker acceptance, not a packaged GUI/native-device run.

Final checks pass: **56 frontend suites / 371 tests**, **971 Rust tests / 27 ignored**, all-target Clippy with `-D warnings`, formatting and the frontend build. The library contributes 628 passed/16 ignored; the cleanup/safety integration target contributes 48 passes. Existing non-fatal CleanupPage effect and Vite bundle-size warnings remain. Ignored tests are not passes. The first frontend check in this pass stopped at formatting because a benchmark JSON file was still being generated. It was formatted after capture before final checking; no test failure was hidden by that initial stop. The active [implementation tracker](../implementation-progress.md) includes remaining acceptance work.

The source-size record was reconstructed against the saved initial dirty-tree patch and untracked archive before updating current counts. Added recovery behavior and regression tests increase the selected source count; shared code reuse is not an overall codebase reduction claim.

## Files changed in this pass

- `src-tauri/src/asr/python_setup.rs` and `src-tauri/src/lib.rs`: versioned metadata command, immutable terminal state, post-probe status capture and the bounded acceptance health assertion.
- `src/services/speechSetup.ts`, its tests, `src/services/tauriApi.ts` and `src/types/index.ts`: owned reattachment, transient polling recovery, ordered native observations and IPC/types.
- `src/components/SpeechDependencySetup.tsx`, `PythonRuntimeSetup.tsx` and their tests: lifecycle ownership, automatic observation, cancellation/readiness state and legacy recovery. `Onboarding.test.tsx` now waits for the resulting asynchronous updates.
- `src-tauri/src/formatting/normalizers.rs`, `formatting/punctuation.rs` and `src-tauri/tests/dictation_polish_quality.rs`: conservative reported interjections, quotation endings and literal-safety integration tests.
- The active tracker and versioned verification reports/JSON: source hashes, model/runtime identity, 200 actual-model pairs, offline pack results and honest remaining limits. Prior source checkpoints are retained separately.

## Scope and remaining work

The [paired transcription review](refinement-2026-10-23/reported-speech/README.md) retains 200 actual installed-model pairs after the quotation fixes. Earlier raw results remain historical, with separate source hashes.

No final visual inspection was possible because browser use is disabled in Work settings. No browser bypass was used. The full native runtime-pack matrix, real microphone/shortcut/text insertion journeys, Windows/macOS filesystem/process behavior, signing/quarantine, real disk-full/crash behavior, superseded-generation cleanup and outstanding dependency advisories remain acceptance gaps. Failure to persist a changed offline policy can still leave a running Python sidecar with an older policy; the existing queued unload attempt does not prove immediate cross-process revocation.

No model downloads, commit, push, PR or release were performed.
