# Independent GLM review — Ponytail refactor (2026-10-22)

- Reviewer: OpenCode `opencode/glm-5.3-flash-free`, agent `plan`, standalone session
- Reviewed diff: `git diff 8d9091138aaa9944024936d5cdba031a031be0c0` on branch `refactor/ponytail-audit`
- Baseline: commit `148ae33`; the stash commit above is the user's pre-existing WIP checkpoint
- Mode: read-only; the reviewer edited nothing

## Verdict

**PASS** — no must-fix or should-fix findings.

## Pre-existing issue confirmed

`src-tauri/tests/long_form_tests.rs:16` imports
`reflow_lib::hotkey::hook::{HookMode, HotkeyStateMachine}`, which the user's WIP removed.
The target fails `cargo check --tests` before and after this refactor and is not part of
the 19-file refactor diff. Library and all other integration targets pass.

## Findings

### Must fix

None.

### Should fix

None.

### Nice to have

1. `src-tauri/src/audio/resampler.rs` — `downmix()` is only safe when the caller has
   already handled `channels <= 1`; a future caller passing 0 channels would panic in
   `chunks_exact(0)`. The reviewer suggested a one-line contract comment.

Disposition: comment added by the lead. No behavior change.

## What the reviewer checked

- Every removed symbol (`push_pcm_s16le`, `set_silence_timeout_ms`, `get_qwen3_dir(_for)`,
  `supports_load`, `reason_codes`, `has_error`, `isAsrOnly`, `findReason`,
  `isDictationLanguage`, `processingModeForCleanup`) has zero remaining references.
- Python's removed `detected_language`, `unprocessed`, and `load_started_at` were
  write-only; the Rust `get_detected_language` path reads the `language` field from the
  stop-stream response, which Python still returns.
- `_record_load_failure` is an exact extraction of the duplicated handler; the different
  `load_model_blocking` payload was correctly left alone.
- `response_error` preserves the old status/fallback semantics; the new unit test pins it.
- Resampler mono path keeps the `channels == 0` guard; file-decode cursor no longer copies.
- `createEventScope` handles late subscription and post-dispose events equivalently.
- `tierState` is identical to both local copies; `SettingsView` still passes
  `onRemoveRuntime` only to `ModelPage`.
- `process_basename` and the Python log-path collapse are behavior-identical.
- The language catalogue now parses once and `resolve_language_name` shares it.

## Checks the reviewer ran

- `cargo test --lib` — 566 passed.
- `cargo clippy --lib --bins` — clean.
- `cargo check --tests` — only the pre-existing `long_form_tests` E0432.
- `npm test` — 314 passed, 1 failed in WIP `HotkeyPicker.test.tsx`; the reviewer
  established it is untouched by the refactor and flaky (a later full run passed 315/315).
- Python selftest with the managed runtime — all cases ok.
- Grep sweeps for removed symbols, IPC keys, and `onRemoveRuntime` threading.

## Not verified by the reviewer

- Live model load / real sidecar round-trip (no GPU model run).
- Reproducing the WIP HotkeyPicker failure on the pure stash tree (would require
  changing the worktree).

## Second read-only check (final state)

A second standalone GLM session reviewed the final tracked diff after the contract
comment and documentation were added.

**VERDICT: PASS.** No new findings. It re-ran `cargo test --lib` (566 passed),
`npm test` (315/315), and the Python selftest via the managed runtime (exit 0), and
re-checked the removed symbols, `process_basename` equivalence, `_record_load_failure`,
`response_error`, and the `downmix` caller guard. It confirmed the only delta since the
first review was the comment plus documentation-only files, and that
`long_form_tests.rs` remains a pre-existing failure untouched by the diff.
