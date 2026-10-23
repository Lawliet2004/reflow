# Historical goal: Reflow Ponytail refactor with independent GLM verification

This is the preserved 22 October audit checkpoint, not the current goal status.
The active first-run reliability and transcription implementation is tracked in
[implementation-progress.md](docs/implementation-progress.md). The historical
Rust compile blocker below has since been fixed and full locked tests/Clippy run.
Native release evidence and the other current requirements remain tracked there.

- Status: blocked
- Created: 2026-10-22
- Updated: 2026-10-22

## Objective

Reflow keeps every existing feature and all current automated checks while its unnecessary duplication, abstraction overhead and dead weight are reduced, and the resulting diff is independently reviewed by a separate GLM-5.3 Flash OpenCode session with every confirmed defect corrected before a final verification report.

## Success criteria

- [x] npm run check passes (two consecutive green runs; a pre-existing WIP HotkeyPicker test is flaky ~2/10 and can still fail)
- [x] npm run build passes
- [ ] cargo test --manifest-path src-tauri/Cargo.toml passes — blocked by pre-existing WIP long_form_tests.rs compile error
- [ ] cargo clippy --all-targets -- -D warnings passes — same blocked target; lib/bins clippy is clean
- [ ] python model-runtime/qwen3_asr_runtime.py --selftest exits 0 — `python` is absent and system python lacks numpy; managed runtime passes all 70 checks
- [x] A separate OpenCode session using opencode/glm-5.3-flash-free reviews the diff read-only and its report is saved in docs/glm-review-refactor.md
- [x] Every confirmed GLM defect is fixed and re-verified, or explicitly reported as FAIL/UNVERIFIED
- [x] Final report states measured or NOT VERIFIED for runtime, Windows, Android and performance claims

## Non-goals

- Rewriting working modules or changing public Tauri IPC / LAN API contracts
- Changing features, privacy controls, migrations, or user settings/data
- Removing code solely because a local text search found no callers
- Benchmark claims without before/after measurement under equivalent conditions

## Constraints

- Pre-existing dirty worktree must be preserved byte-for-byte except where the refactor intentionally touches the same lines
- No destructive cleanup, history rewrite, forced reset, or push
- Only one agent may edit the worktree at a time; GLM edits only after DeepSeek stops
- Ponytail Full is the refactoring mode; plugin 5.3.2 installed globally

## Milestones

### M1 — Baseline and repo map

- Status: complete

Acceptance:

- [x] Git commit and dirty state recorded and backed up
- [x] Baseline checks run with real output captured
- [x] Audit findings ranked by risk in the session audit

### M2 — Low-risk simplification batch

- Status: complete

Acceptance:

- [x] Each change has callers traced and a regression test where non-trivial
- [x] npm run check and npm run build pass

### M3 — Rust backend simplification batch

- Status: blocked

Blocker: pre-existing user WIP removed `hotkey::hook::{HookMode,HotkeyStateMachine}` and `tests/long_form_tests.rs` still imports them, so the full `cargo test`/`--all-targets` build fails independently of this refactor. Library and all other integration targets pass. Not edited to avoid touching user work.

Acceptance:

- [ ] cargo test and clippy -D warnings pass (blocked by long_form_tests.rs)
- [x] No IPC command, migration or lifecycle behavior changed

### M4 — Independent GLM review

- Status: complete

Acceptance:

- [x] GLM report saved with PASS/FAIL/UNVERIFIED findings
- [x] Findings independently evaluated against code

### M5 — Corrections and full verification

- Status: complete

Acceptance:

- [x] All automated checks rerun green (baseline failures excepted)
- [x] Runtime smoke or explicit NOT VERIFIED per untestable platform

### M6 — Final report

- Status: complete

Acceptance:

- [x] Report includes diff, line counts, test counts and remaining risks
- [x] GOAL.md closed truthfully

## Current focus

None — closed as blocked on the pre-existing `long_form_tests.rs` compile failure and the unavailable exact `python` command.

## Decisions

- 2026-10-22 — Goal created.

## Log

- 2026-10-22 — Tracker created.
- 2026-10-22 — Ponytail 5.3.2 installed and loaded; full mode active. Baseline at 148ae33 on branch refactor/ponytail-audit; user WIP backed up (stash 8d90911, patch + untracked tarball in /tmp/opencode/reflow-wip-backup).
- 2026-10-22 — Baseline: npm check 313/314 (one flaky HotkeyPicker failure), build pass, cargo lib 564 pass/14 ignored, all integration targets pass except long_form_tests (stale WIP imports), clippy lib/bin pass, Python selftest pass under managed runtime (system python lacks numpy).
- 2026-10-22 — DeepSeek refactor batch: 19 files, +125/-221 (net -96). Duplicate load-failure/log-path/sidecar-error/tierState logic collapsed; dead Python stream state and seven unused Rust helpers plus four unused frontend helpers removed; language catalogue cached; resampler avoids a mono copy. Verified npm check 315/315 twice, build, cargo lib 566/566, clippy lib/bin, Python selftest 70/70, Python unittest 39/39, 326 integration tests across 40 targets except long_form_tests.
- 2026-10-22 — Independent GLM-5.3 Flash review (opencode/glm-5.3-flash-free, plan agent, standalone) returned PASS with no must-fix or should-fix findings; one nice-to-have contract comment applied. Report in docs/glm-review-refactor.md.
- 2026-10-22 — Second standalone GLM read-only review of the final diff returned PASS with no new findings; it re-ran cargo lib tests 566/566, npm test 315/315, and the managed-runtime selftest (exit 0).
- 2026-10-22 — Runtime smoke: built debug binary, `--status` succeeded, portable app stayed running, created history.db and spawned the Python sidecar. No microphone, hotkey, live transcription or model-load round-trip was exercised. Windows/Android/GPU inference NOT VERIFIED.
- 2026-10-22 — Closed as blocked: full cargo test/clippy --all-targets still cannot compile the untouched WIP long_form_tests.rs; exact `python` command unavailable. Pre-existing WIP HotkeyPicker test is flaky (2/10 standalone runs).
