# Intelligent formatting verification — 2026-10-22

The working tree already contained the formatting setting, selector, prompt extension,
shared preview/dictation processing, and formatted rewrite validator when this task
started. Those changes were retained and reviewed. This pass extends technical
repair recognition to `no wait`, with optional `actually` and `use`, adds bare `no`
repairs between values in the same closed class (such as weekdays), and adds
regression tests and an optional cold/warm evaluation.

## Pipeline and compatibility

Stage 1 still owns fillers, punctuation, replacements and vocabulary. The shared
`session::postprocess_with_context` path passes the effective formatting preference
to the existing llama.cpp client. Formatting instructions vary in the user turn;
the system prefix remains stable. Structured formats receive the whole transcript
in one pass, with existing context/deadline checks; overflow falls back rather than
structuring unrelated chunks independently. Validation precedes insertion. Raw,
literal Coding, and No LLM selections remain protected. Explicit mode transformation
instructions retain their existing precedence. History retains raw input.

The existing UI exposes Off, Auto, Natural, Technical, Brainstorm, Meeting Notes,
and Steps, with bounded custom presentation guidance and fallback reporting. Off
is the migration default. Enabling formatting does not automatically override a
No LLM selection; choose an installed local Qwen model through the existing selector.

Relevant feature files already modified in the incoming tree:

- `src-tauri/src/settings/config.rs`, `settings/expansion.rs`
- `src-tauri/src/session.rs`, `commands/mod.rs`
- `src-tauri/src/rewrite/prompt.rs`, `client.rs`, `safety.rs`
- `src/components/settings/CleanupPage.tsx`, `CleanupPage.test.tsx`
- `src/types/index.ts`, `src/services/tauriApi.ts`

Files changed or added in this pass:

- `src-tauri/src/rewrite/safety.rs`
- `src-tauri/src/formatting/backtrack.rs`
- `src-tauri/tests/adaptive_format_live.rs`
- this report

## Before/after fixtures

These are validator fixtures, not observed model generations. Passing them proves
that these edits are accepted by the guard, not that a model reliably produces them.

A. “hey so um I was thinking we could meet tomorrow maybe around five pm … bring
your laptop if possible” → “Hey, so I was thinking we could meet tomorrow, maybe
around five pm, and discuss the project. And yeah, bring your laptop if possible.”

B. “build a backend using node js no wait actually python fast api …” →

Build a backend with the following requirements:

- Framework: Python FastAPI
- Database: PostgreSQL
- Authentication: JWT
- Security: Password hashing
- Caching: Redis

C. “I want an ai browser that's lightweight uses less RAM and maybe it can open
websites search stuff do tasks and I think privacy should be important too” →

Ideas

- I want an AI browser that's lightweight and uses less RAM.
- Maybe it can open websites, search stuff, and do tasks.
- I think privacy should be important too.

The full requested C input, including “do tasks”, is also included in the live evaluation.

D. “rahul will finish frontend by friday priya will do the api integration testing
is saturday and whole thing before monday” →

Meeting notes

- Rahul will finish frontend by Friday.
- Priya will do the API integration.
- Testing is Saturday.
- Whole thing before Monday.

E. “improve my speech to text application … keep it offline and don't change my
meaning” →

Technical requirements

- Improve my speech-to-text application.
- Make transcription faster.
- Use a small LLM to correct mistakes and format the text automatically.
- Recognize code and commands.
- Support normal writing, coding, and meeting notes.
- Keep it offline, and don't change my meaning.

## Verification

- `npm run build`: passed; Vite reports a bundle size warning.
- `npx --no-install vitest run`: 52 files, 320 tests passed.
- Full offline Cargo suite passed after the repair changes; the updated five-scenario fixture test also passed separately.
- Updated Rust library suite: 590 passed, 15 ignored.
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`: passed.
- Live evaluation compiled, but its prerequisite check failed: no GGUF at
  `/home/papan/.local/share/reflow/models/flow/Qwen3.5-0.8B-Q4_K_M.gguf`.
  No model or runtime installation was attempted.

## Live evaluation and limits

Run `cargo test --manifest-path src-tauri/Cargo.toml --offline --test
adaptive_format_live -- --ignored --nocapture` where existing model/runtime files
are available. `REFLOW_APP_DIR` can point at an existing Reflow data directory;
`REFLOW_LLAMA_BIN` can identify an existing llama-server binary.
`REFLOW_EVAL_MODEL=qwen3.5-2b` selects the existing larger alternative.

The evaluation uses one runtime on CPU, avoiding a second GPU residency. It emits
JSON records for two passes over all five requested inputs: accepted rewrite use,
fallback reason, output, cleanup/rewrite/total processing time, token counts and
prompt/decode timing when llama-server supplies them. It is a measurement harness,
not an independent semantic quality score. Pass zero starts without explicit cache
warming; later cases already share the prefix, so only its first case is cold.

No inference benchmarks or release-to-insertion measurements are available here.
Microphone, graphical preview, actual paste, Windows behavior and 0.8B semantic
reliability were not verified live. Conservative lexical validation may reject
valid paraphrases or broader self-corrections; fallback preserves the cleaned
transcript. Automatic checks cannot prove semantic equivalence.

## Follow-up edge-case audit

A subsequent audit found a Raw/Verbatim bypass through custom mode transformation
instructions. The shared postprocessor now blocks all such tasks at raw cleanup,
including translation; literal Coding remains protected. The dictation, preview,
and retry runtime gates also avoid loading refinement for these raw-mode tasks.
Explicit assistant/command actions retain their separate behavior.

New regressions cover raw cleanup combined with custom instructions/translation,
and the actual final-output pipeline using a mock local server. The latter accepts
structured meeting notes and verifies deterministic fallback for an altered date,
an invented testing owner, and an omitted deadline. These are controlled response
tests, not evidence of live Qwen accuracy.

Follow-up library suite: 591 passed, 15 ignored. The new final-output integration
test passed. The full follow-up Cargo suite also passed. Rust formatting and whitespace checks passed. Live-model and desktop
limitations above still apply; this feature is not claimed to handle every possible
utterance or to guarantee perfect semantic preservation.
