# Feature expansion verification

For the subsequent whole-codebase bug fixes and Windows installer validation, see [Whole-codebase audit](WHOLE_CODEBASE_AUDIT.md). Counts below describe the earlier feature-expansion run; the later audit records its own final checks.

The implementation follows `feature-expansion-prompt.md` in phase order. Existing uncommitted work is preserved; no commits or releases are created by this implementation.

## Phase 0 — Foundations

- Settings schema 5: additive defaults, legacy hotkey migration and patch translation, nested hotkey merging, built-in modes, output actions, context options, and input limits.
- History schema 5: transactional migration for command input, kind, source, pins and tags; all read/insert paths include the new columns.
- Session intent, resolved mode and opted-in local context are held separately from capture provenance.
- Task prompts keep instructions and context in the final user turn; the existing cleanup prefix and strict safety gate are unchanged.
- Local task completion shares runtime/context accounting; transformative output has a separate validation gate.
- Multiple shortcut bindings and Windows modifier-only state machines; Escape cancellation passes through the Windows hook.
- Output dispatcher supports paste, paste-and-send, persistent copy, HUD, file append and configured shell commands. Text in command placeholders is passed as shell data; without a placeholder it is written to stdin.
- Selected-text capture preserves the clipboard snapshot and serializes with injection.

Validation logs are written alongside this file. Baseline frontend lint passed; the baseline check stopped on formatting of the supplied prompt. The prompt has since been formatted without changing requirements.

## Remaining phase checkpoints

- Phase 0: full Cargo suite, all-target Clippy, and frontend check passed. Logs: `feature-expansion-phase0-full-final.log`, `feature-expansion-phase0-frontend-final.log`.
- Phase 1: full Cargo suite passed. All-target Clippy passed after test initialization fix. Frontend checks now pass (122 tests). An isolated HTTP regression proved missing context accounting on tokenizer fallback; all eight regression tests now pass, preserving the cacheable prefix.
- Phase 2: full Cargo suite and all-target Clippy passed. Fifteen synthetic file/note/correction tests cover decoding, timestamps, cancellation and corrections; adjacent-transposition failure was reproduced and fixed. Native installed-runtime tests also passed.
- Phase 3: implemented. Focused storage/encryption (11), network/transfer/Python, automation/portable and options checks pass. The initial full run reproduced smiley spacing loss in punctuation cleanup; the fix passes all five option regressions. Final full checks include the integrated implementation.
- Phase 4: meeting capture, summaries and assistant proposals implemented; ten focused regression cases included in final suite. Optional diarization intentionally omitted under the prompt’s explicit allowance; nullable speaker metadata retained.

Native input/focus and clipboard behavior require desktop integration testing on each supported OS; pure selection-change and output-dispatch tests exercise the deterministic paths.

## Phase 1 and 2 behavior

- Snippets run once after formatting, glossary and optional local generation, preserving exact stored line breaks and casing. Triggers are also ASR vocabulary.
- Modes expose optional presets, per-mode shortcuts, triggers, output, local context and translation. Command editing preserves the selection on generation failure; no selection falls back to normal dictation. Translation and custom instructions combine in the task user turn.
- Audio file decoding uses Symphonia 0.5.5 for supported local containers/codecs (hound is WAV-only). [Primary API documentation](https://docs.rs/symphonia/0.5.5/symphonia/). Imports require confirmation above 100 MB and reject decoded duration beyond two hours. Timestamped segments use independent ASR streams, not invented timings. File imports preserve original ASR content and subtitle alignment; they do not paste into another app.
- The assistant is stateless and defaults to a persistent, dismissible response HUD. Notes have manual entry, voice capture, search, pins, copying and Markdown export. Edited history entries produce bounded spelling/case/hyphen suggestions, with persisted dismissal pairs.
- Browser verification saved a Jira preset with all context permissions off and inspected file/Notes controls. Screenshot: `verification-images/expansion-modes.png`. Browser preview has no native model, microphone, hotkey or clipboard bridge; native physical flows are not claimed as verified.
- Remaining native checks: file picker focus behavior, physical command replacement/clipboard restoration, multi-line response HUD scrolling/dismissal, and local live microphone intents on each supported OS.

## Phase 3 integration

Encryption uses schema 6 for its transactional state/verifier manifest. Settings retain additive schema-v5 defaults; history v4 migrates through v5 and then v6 in place. Stats aggregate plaintext metadata; transcripts/selection/PCM/subtitle JSON are authenticated encrypted payloads. Original databases are preserved on missing-key recovery. Advanced controls do not create an automation credential merely by opening the page. Downloads default to airplane mode; explicitly turn it off to install pinned artifacts. Outbound redirects are host-restricted, and local inference redirects are disabled.

Windows ducking runs on a dedicated COM thread and restores unchanged session volumes; see [Microsoft IAudioSessionManager2](https://learn.microsoft.com/en-us/windows/win32/api/audiopolicy/nn-audiopolicy-iaudiosessionmanager2) and [ISimpleAudioVolume](https://learn.microsoft.com/en-us/windows/win32/api/audioclient/nn-audioclient-isimpleaudiovolume). Battery detection uses [GetSystemPowerStatus](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-getsystempowerstatus). These native integrations are compiled on Windows; physical audio-volume and AC/battery transitions still require manual desktop checks. Mid-recording microphone failover remains out of scope.

## Phase 4 integration

See [meeting and assistant guide](meeting-and-assistant.md) for Windows loopback, Linux monitor routing, platform limits, summaries and proposal activation. Both native capture streams resample to 16 kHz and feed a bounded mixer positioned by device capture timestamps; absent playback callbacks are silence, and late samples cannot be shifted into later speech. Whole-session peak/sample tracking preserves earlier speech after a quiet tail overflows the retained audio buffer. Meeting captures skip ducking and injection, retain separate provenance and use the normal ASR/history path. Summary tasks use at least a 4096-token runtime context without changing stored preferences. Chunks have natural boundaries and bounded calls/deadline; failures preserve the original. The strict assistant parser only recognizes whole SEARCH/NOTE lines; the HUD requests explicit activation and never runs model output as shell.

The final privacy pass also reproduced a missing-key recovery write hazard. With encryption requested, private writes are blocked when encryption is unavailable, including recovery; restoring the OS key or explicitly disabling encryption is necessary to persist new text. The original database remains untouched. This is covered by a new storage regression. WAL checkpoint/truncation prevents pre-migration plaintext journal frames from surviving encryption enablement.

Browser verification: light/dark Advanced settings, meeting opt-in and Home controls were inspected in the actual browser; no warnings/errors were emitted. Screenshots: `verification-images/expansion-advanced-light.png`, `verification-images/expansion-advanced-dark.png`, `verification-images/expansion-home-meeting.png`, `verification-images/expansion-recording-options.png`. Preview changes are in browser memory and do not modify native user preferences or create credentials.

## Final acceptance status

- Frontend: `npm run check` passed: ESLint, Prettier, TypeScript and 130 tests across 34 files. Log: `feature-expansion-final-frontend.log`.
- Python runtime: `python model-runtime/qwen3_asr_runtime.py --selftest` passed. Log: `feature-expansion-python-selftest.log`. The Python runtime/network-policy unittest suite also passed (17 cases, including three new network-policy cases): `feature-expansion-network-python-final.log`.
- Rust: full suite passed (556 tests, one pre-existing ignored hardware calibration test). All-target Clippy passed with warnings denied; formatting passed. Logs: `feature-expansion-final-rust.log`, `feature-expansion-final-clippy.log`. The newly added real-model summary task also passed against installed llama-server/GGUF with a 4096-token context (`feature-expansion-summary-live.log`), adding one regression beyond the full-suite run.
- Settings v4 migration, history v4 migration, additive protocol/serde defaults, task-prefix invariance, encrypted reads/writes and cancellation are tested. Android changes are limited to existing/additive server contracts.
- Offline audit: outbound application downloads use pinned HTTPS host guards and the journal. Native/Python inference uses local artifacts; local HTTP redirects are disabled. The frontend has no external fetches. LAN API requests remain authenticated; browser search/releases only occur on explicit clicks.
- Not physically verified: network-disconnected/hosts-blackhole desktop session, real mic+loopback capture, native clipboard/focus, volume restoration, battery transitions, Linux monitor/Wayland routing, and OS-keyring integration. Their deterministic contracts and Windows builds are checked; this record does not claim physical hardware coverage.

## Independent review

The code-review-and-quality pass found and reproduced three integration errors: missing encryption enforcement on transcript retry, default summary context exhaustion, and earlier meeting speech being canceled after a silent retained tail. All three RED logs are retained (`feature-expansion-review-red.log`, `feature-expansion-review-phase4-red.log`), and all 21 focused storage/Phase4 cases now pass (`feature-expansion-review-green.log`). Device capture timestamps plus a monotonic source cursor replace callback-arrival positioning, with a deterministic jitter regression. The reviewer rechecked the fixes and reported no remaining actionable finding in the reviewed paths.

All implementation phases are complete under the prompt’s allowed diarization omission. No commits or releases were created. Verification did not download models, create credentials, or change native settings/history databases; browser controls used preview memory.
