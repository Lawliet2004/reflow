# Object-preserving Auto cleanup

> **Later source:** [Paragraph-preserving corrections](../paragraph-preservation/README.md) fix a subsequently confirmed Stage 1 paragraph-loss bug and add live explicit-break checks. This checkpoint's raw measurements remain historical evidence.

> The measured refinement pipeline is unchanged by the subsequent [ASR policy/lifecycle update](../../network-policy-revocation-2026-10-23.md). That report and the active tracker hold the latest whole-worktree verification counts.

23 October 2026. The overall cross-platform reliability/transcription objective remains incomplete. This change uses the installed Qwen3.5-0.8B and existing llama-server; no model weights were downloaded.

## Diagnosis and change

Stage 1 already correctly resolved the dictated React → Svelte repair. With the preceding prompt, actual inference produced `Use Svelte and connect to PostgreSQL instead of MySQL.` It omitted the object **it**. The safety gate correctly rejected the candidate and returned the less polished deterministic source. This was a model/instruction failure, not a reason to relax the validator.

Only `src-tauri/src/rewrite/prompt.rs` changed in this pass. Ordinary high-intensity Auto cleanup now uses one shorter final-user instruction that explicitly retains pronouns and their objects. It replaces the redundant general cleanup plus Auto-presentation guidance. The shared system prefix is byte-identical. No classifier, second inference pass, model, decoding-default, context or safety-threshold change was introduced.

The existing compact requirement-list instruction remains for plain Auto cleanup. It previously could override an explicitly selected register or dictation mode. Both compact branches now preserve decisive/email/chat registers, notes/email/developer modes and custom or explicit presentation preferences. Two regression tests cover compact object guidance, stable system content and that precedence.

## Actual installed-model results

| Context / decoding     | Applied | Coarse contracts | Median / p95 total ms | Peak server RSS MiB | Device VRAM MiB |
| ---------------------- | ------: | ---------------: | --------------------: | ------------------: | --------------: |
| 2048                   |   43/50 |            50/50 |           260.5 / 406 |               761.6 |            1239 |
| 4096                   |   43/50 |            50/50 |           262.5 / 424 |               761.5 |            1265 |
| 8192                   |   43/50 |            50/50 |             266 / 409 |               761.3 |            1317 |
| 4096, temperature 0.15 |   42/50 |            50/50 |             263 / 406 |               761.4 |            1265 |

Every context was checked against live `/props`. Adjacent raw JSON retains all 150 deterministic and 50 temperature-0.15 pipeline trials, prompt/decode timing, token counts, fallback reasons and memory observations. Source/model/executable hashes are in `provenance.json`. Prepared Stage 1 inputs and all safety/context/decoding sources are unchanged from the [preceding source checkpoint](../reported-speech/README.md).

At every deterministic context, **49 of 50 final outputs are byte-identical** to that checkpoint. The changed pair is:

```text
Input:   use react no actually svelte and connect it to postgres not mysql
Before:  Use svelte and connect it to postgres not mysql.
Current: Use Svelte and connect it to PostgreSQL instead of MySQL.
```

The final choice, exclusion and object are retained. Applied rewrites rose from 40/50 to 43/50 and safety fallbacks fell from 10 to 7. The other two newly accepted outputs (`repeated_words`, `actually_emphasis`) already match their faithful deterministic text; this reduces false rejection reporting without claiming additional visible polish. No unsafe retained-output change was identified in this bounded paired review; it is not a general semantic-equivalence proof.

Median rendered prompt tokens fell from **393.5 to 349.5** in this corpus. At 4096 the preceding median/p95 latency was 272.5/444 ms; the current observation is 262.5/424 ms. Desktop load was not isolated, so the latency difference and approximately 19 MiB higher observed RSS cannot be attributed solely to this instruction change. Device VRAM includes the resident app at its saved 1024 context. One temporary benchmark server ran at a time and was shut down. Existing saved preferences and the 4096 default for new/unset values remain unchanged.

Temperature 0.15 again loses the technical correction to a safety fallback, removes the greeting comma in the ordinary paragraph and only changes a comma in the topic-change case. Deterministic decoding remains.

## Controlled probe and validation scope

Before editing production source, the same 50-input production pipeline was run twice through the resident 1024-context server: unchanged control and a proxy replacing only the final user instruction. Both chat-template rendering and generation requests were transformed, so token accounting sees the measured instruction. No extra model process was started. The control applied 40/50; the candidate applied 43/50, with all coarse contracts retained. `controlled-1024.json` preserves both sets and the proxy-script hash. The ten earlier raw candidate diagnostics are explicitly labeled as not safety-validated in `raw-diagnostic.json`.

The 20 prompt-unit tests and full locked Rust suite pass: **973 passed, 27 ignored** (library 630 passed/16 ignored). All-target Clippy with `-D warnings` and Rust formatting pass. Ignored tests are not passes. Full checks and remaining release work are recorded in [the active tracker](../../../implementation-progress.md). The 371-test frontend/build evidence belongs to unchanged frontend sources from the setup-recovery pass; this prompt-only change adds no TypeScript or IPC contract.

## Remaining acceptance gaps

Eight substantive cases remain: `ordinary_paragraph`, `email`, `requirements_modifiers`, `meeting_actions`, `two_topics`, `no_wait_request`, `long` and `long_corrected_requirements`. SQLite/Python casing is still imperfect. The unchanged seven fallbacks remain explicit; some protect against actual model content/literal/identifier changes. The 50/50 coarse score does not prove publication-quality sentence boundaries, semantic formatting or universal accuracy.

Native Windows/macOS and physical Linux installer/device acceptance, broader backend fault coverage, dependency advisories and final visual verification remain outstanding. No release was published.
