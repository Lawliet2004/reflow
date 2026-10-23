# Paragraph-preserving speech corrections

> The subsequent [policy-recovery diagnostic update](../../policy-recovery-message-2026-10-23.md) holds the latest whole-worktree verification and offline-pack check. This measured refinement pipeline remains unchanged.

23 October 2026. The full Windows/macOS/Linux reliability and transcription goal remains incomplete. This checkpoint fixes demonstrated Stage 1 content-structure loss using the existing cleanup infrastructure and installed Qwen3.5-0.8B. No model weights were downloaded.

## Confirmed failures and fix

The previous 50-input review missed a structural error in `long_corrected_requirements`. A correction in its second paragraph, `Use Node.js no wait use Python FastAPI`, rebuilt the entire preceding text with `split_whitespace().join(" ")`. The first blank line disappeared before inference. Consequently the safe fallback also returned two paragraphs rather than the original three. Reconstructing the repaired comparison source could also weaken the validator's existing newline check.

Correction detection now keeps token byte offsets, restricts the abandoned phrase and replacement match to their current line, and copies the preceding source slice. Earlier paragraphs, Unicode text and literal whitespace survive. Repair-marker whitespace no longer matches across explicit newlines, including value corrections, `scratch that` and `no seriously`. A period terminating a replacement such as `Svelte.` no longer prevents recognition; internal dots in `Node.js` remain intact. Ambiguous repairs across explicit lines are retained for contextual handling rather than deterministically deleting text.

The two added backtrack test groups failed before the fix. They cover later-paragraph repairs, multiple repairs, a repeated pronoun anchor in an earlier paragraph, Unicode prefixes, exact code whitespace and nine line-boundary cases. An integration regression verifies complete Stage 1 output, accepts the faithful correction and rejects flattened output from both the original and prepared source. No safety threshold, system prompt, context budget or decoding default changed.

Changed source files:

- `src-tauri/src/formatting/backtrack.rs`: line-bounded detection and original-prefix preservation.
- `src-tauri/tests/dictation_polish_quality.rs`: cleanup/safety integration regression.
- `src-tauri/tests/refinement_benchmark.rs`: record `breaks_preserved` and include it in the layout contract.

## Actual installed-model comparison

The same 50 inputs were run at every context through production `postprocess_transcript`, then at temperature 0.15/4096. The harness checks live `/props` active per-slot context. Raw results and scoped source/model/executable hashes are adjacent; the paired source is [object-retention](../object-retention/README.md).

| Context / decoding     | Applied | Coarse contracts | Explicit breaks | Median / p95 total ms | Peak server RSS MiB | Device VRAM MiB |
| ---------------------- | ------: | ---------------: | --------------: | --------------------: | ------------------: | --------------: |
| 2048                   |   43/50 |            50/50 |           50/50 |           263.5 / 406 |               745.6 |            1239 |
| 4096                   |   43/50 |            50/50 |           50/50 |           266.5 / 411 |               745.7 |            1265 |
| 8192                   |   43/50 |            50/50 |           50/50 |             266 / 420 |               745.9 |            1317 |
| 4096, temperature 0.15 |   43/50 |           49/50* |           50/50 |             268 / 428 |               745.5 |            1265 |

At each deterministic context, **49 of 50 prepared and final outputs are byte-identical** to the preceding checkpoint. The changed long case retains all three paragraphs/four newline characters, versus two paragraphs/two newlines previously. It still falls back explicitly because the model changes a literal. The Node.js → Python and Thursday → Friday repairs and all protected content remain. Seven deterministic fallbacks remain; none is counted as an applied model rewrite.

An excerpt from the changed final text previously joined the topics:

```text
Maybe we can add local AI assistance later but that is not a launch requirement. Use Python FastAPI for the service and PostgreSQL for the database not MySQL.
```

The current final text retains the original separation:

```text
Maybe we can add local AI assistance later but that is not a launch requirement.

Use Python FastAPI for the service and PostgreSQL for the database not MySQL.
```

Reapplying the new explicit-break contract to the old raw outputs scores **49/50**, despite their previously reported 50/50 coarse score. Newline counts still cannot prove that every boundary is in the right place; the focused exact regressions and paired review supplement that coarse check.

*The temperature run's coarse false failure is `technical_negation`: the unchanged substring oracle expects `not React` and `not MySQL`; the model says `instead of React` and `instead of MySQL`. Both exclusions are retained. Raw missing fields remain recorded. This run gains one faithful paraphrase over deterministic decoding but does not establish a material quality improvement; repeated temperature runs also vary. Deterministic decoding remains selected.

Median prompt tokens remain 349.5. At 4096 the preceding median/p95 latency was 262.5/424 ms and peak RSS 761.5 MiB. Desktop load was not isolated; neither the small timing difference nor lower observed RSS establishes an optimization. Device VRAM includes the resident app at its saved 1024 context. Its preference was preserved; one temporary benchmark server ran at a time and was shut down. The existing 4096 default for new/unset preferences is unchanged.

## Checks and remaining work

The focused backtrack suite passes 20 tests; the dictation integration target passes 49. Full locked Rust checks pass **982 tests, 27 ignored** across 45 targets, including installed-model integration; library **638 passed/16 ignored**. Ignored tests are not passes. All-target Clippy with `-D warnings`, Cargo formatting, repository formatting and `git diff --check` pass; their evidence is recorded in `provenance.json`. Unchanged frontend sources retain the preceding 371-test/build evidence; no frontend or IPC field changed here.

Eight substantive writing cases remain: `ordinary_paragraph`, `email`, `requirements_modifiers`, `meeting_actions`, `two_topics`, `no_wait_request`, `long` and `long_corrected_requirements`. Sentence boundaries, semantic formatting and SQLite/Python casing still need work. This fix preserves explicit structure; it does not finish publication-quality cleanup. Coarse scores are not general factual-accuracy evidence.

Native Windows/macOS and physical Linux acceptance, the full runtime-pack matrix, broader crash/disk/process faults, 15 allowed Rust advisory warnings and final visual verification remain open in [the active tracker](../../../implementation-progress.md). Earlier ASR policy/AppImage/offline-pack evidence is preserved in its own source checkpoint. No release was published.
