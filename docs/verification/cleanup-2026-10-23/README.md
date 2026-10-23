# Cleanup implementation verification — 23 October 2026

Completed the existing cleanup implementation in the working tree. The requested prompt remains byte-identical to the supplied text (excluding the source file's final newline). No settings schema changes, external network requests, or production transcript logging were added.

## Changes in this turn

- Strengthened script validation: any dominant-script change between Latin, Devanagari and Bengali is rejected, as requested.
- Tightened content-word matching: unrelated words such as department/deployment and coffee/coffin no longer pass because of a shared prefix. Small suffix inflections remain accepted.
- Reject a newly introduced repeated four-gram even when the speaker repeated a different four-gram.
- Strip same-line acknowledgement preambles without confusing words such as “surely” with “sure”, or stripping acknowledgements actually spoken.
- Extend the existing deterministic correction pass to restarted commands such as “send it to Rahul no wait send it to Priya”. Ordinary “wait for me” and “actually I think…” remain intact.
- Long unpunctuated ASR text now has boundaries at explicit discourse transitions, including “switching topics” and “last thing”. Topic chunks join with a blank line. Numbered markers, deterministic bullets, and spoken number-word lists stay together; numbered markers cannot become detached sentences.
- Custom modes cannot enable refinement when the selected tier disables the LLM. Explicit translation/assistant actions remain separate existing features; ordinary cleanup never translates or answers dictated instructions.
- Surface the existing reason-code metric in both diagnostics waterfalls, with backward-compatible optional TypeScript typing. The existing rewrite_ms measurement includes planning, local template/token accounting, sequential model requests and validation.
- The opt-in eval prints full fixture inputs and outputs, including long dictations, and reports subprocess failures rather than accepting a partial summary.
- Added regression coverage for the above. Existing mocked request, sampling, token-estimation, skip, fallback and thinking-stripping tests remain in place.

Live dictation, retry and import continue to use the shared postprocessor. Prompt caching, named sampling constants, the 4096-token minimum, script-aware budgeting and the 300-word chunk target were already implemented and were retained.

## Verification

- cargo test --offline -j 2: **1,003 passed, zero failed, 28 ignored**.
- cargo clippy --offline --all-targets -j 2 -- -D warnings: **passed**.
- npm run lint: **zero errors, one pre-existing warning** in CleanupPage.tsx (setState in an effect).
- npm run test -- --maxWorkers=2: **372 passed** across 56 files.
- npx tsc --noEmit, node --check scripts/eval_cleanup.cjs and git diff --check: **passed**.

The first frontend run, launched alongside a heavily parallel Rust build, timed out in 19 tests. With two frontend workers the entire suite passed, and the final run after the diagnostics changes passed all 372 tests. A new numbered-list regression initially failed; correcting sentence detection around numbered markers fixed it before the final Rust run.

## Real model and latency

Command: REFLOW_EVAL_BACKEND=vulkan node scripts/eval_cleanup.cjs

Installed Qwen3.5-0.8B Q4_K_M, pinned llama.cpp b11146 (build 11146, commit 7fe450e19), 4096 active context, Vulkan. Model and executable hashes are adjacent.

Final corpus result: **13/15 (86.7%)**. The strict eval exits with status 1 because the default required rate is 100%; this quality failure is intentionally preserved.

Remaining exact-match failures:

- self_correction: “So I want coffee.” versus “I want coffee.”
- deliberate_repeat_kept: “It was very, very good.” versus “It was very very good.”

Recipient correction/time, accidental repeats, lists, prose, dictated questions/commands, Hindi/Bengali code-switching, long three-topic dictation, and two-word skipping passed. The fixed system prompt and golden expectations were not weakened to increase the score.

Final cleanup-stage latency: **169 ms median, 427 ms mean** across all 15 cases, including the short skipped case and long dictation. Before this turn the same corpus measured 180 ms median and 449 ms mean and passed 11/15. These runs were not controlled performance trials; the observed -11 ms median is not a causal speedup claim. Validation of a 300-word chunk averaged **1.85 ms** per call in a debug build. That validation cost is already included in rewrite_ms, not an extra unaccounted stage. No release-build or end-to-end microphone-to-injection latency measurement was made.

Thinking was probed against the actual installed binary and GGUF template: enable_thinking=false renders an already closed thinking block, whereas true renders an open block. A synthetic completion returned ordinary content and finish_reason=stop, without reasoning content. Production also passes --reasoning off. Full observations are in thinking-verification.txt.

## Limits and review

An indivisible oversized sentence or list may exceed the target chunk length. It stays intact and falls back to deterministic text if it cannot fit the context; the pipeline does not cut inside a spoken list. Silence-segment metadata is not currently passed into the text postprocessor; sentence, line and explicit topic boundaries are used instead. Script checks cannot detect translation between languages sharing a script, and content-word checks remain conservative heuristics rather than a proof of fidelity.

The working tree already contained extensive modified and untracked implementation files before this task. No commits were created, to avoid incorporating unrelated existing work. changes.patch contains only this turn's edits relative to that starting working tree; source-hashes.json records those before/after versions. The patch therefore targets the initial working tree, not a clean repository HEAD.
