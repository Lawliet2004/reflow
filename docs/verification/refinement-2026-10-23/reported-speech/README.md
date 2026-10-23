# Reported speech and quotation punctuation

> Historical source checkpoint. See [object-preserving Auto cleanup](../object-retention/README.md) for the latest pipeline measurements.

23 October 2026. The full cross-platform implementation remains incomplete. These runs use the installed Qwen3.5-0.8B Q4_K_M and existing llama-server; no model weights were downloaded.

## Actual installed-model comparison

| Context / decoding     | Applied | Coarse contracts | Median / p95 ms | Peak server RSS MiB | Device VRAM MiB |
| ---------------------- | ------: | ---------------: | --------------: | ------------------: | --------------: |
| 2048                   |   40/50 |            50/50 |       279 / 409 |               742.5 |            1238 |
| 4096                   |   40/50 |            50/50 |     272.5 / 444 |               742.5 |            1264 |
| 8192                   |   40/50 |            50/50 |     274.5 / 412 |               742.2 |            1316 |
| 4096, temperature 0.15 |   40/50 |            50/50 |       270 / 411 |               742.9 |            1264 |

The harness verified each active per-slot context using `/props`. The 150 deterministic input/output pairs and 50 temperature-0.15 pairs retain prompt/decode timing, token use, fallback reasons and memory observations in the adjacent JSON files. Source, model and executable hashes are in `provenance.json`.

At each context, **49 of 50 deterministic outputs are byte-identical** to the preceding [hardening run](../hardening/README.md). The changed pair is:

```text
Input:   he shouted no no no
Before:  He shouted no no no.
Current: He shouted, "No, no, no!"
```

Stage 1 now produces `He shouted, 'No, no, no!'` before inference. The model changes the enclosure to double quotes while retaining the complete quoted content. Every repeated interjection survives; regression tests reject deleting one or reversing no to yes.

Deterministic decoding remains. This temperature sample matches its coarse score, differs only in requirement-list bullet styling at context 4096, and does not show a consistent quality improvement. Earlier stochastic failures remain recorded in the hardening report. The 4096 default for new/unset preferences and existing saved values are unchanged.

Memory and latency were observed under desktop load. Device VRAM includes the resident app at its saved 1024 context. Only one temporary benchmark server ran at a time, and it was shut down. The lower RSS than the prior run is an observation, not evidence that this punctuation change optimizes model memory.

## Small shared changes

- The existing emphasis normalizer recognizes a complete pronoun + shouted/yelled/screamed + repeated identical no/yes clause. It supplies direct-speech punctuation without deleting words. Longer clauses, mixed interjections, repairs, existing quotations, newlines and technical modes retain their treatment. It does not infer quotations in general prose.
- The shared terminal-punctuation helper no longer appends an extra period after a closed quotation that already contains sentence punctuation. It recognizes paired straight/curly quotation marks, preserves enclosing question punctuation, and does not treat a stray closing mark as a complete quotation.
- Existing literal and semantic safety gates remain active. No prompt, threshold, model, classifier, inference pass or decoding-default change was introduced.

The end-to-end test first exposed the prior `He shouted, 'No, no, no!'.` result; the shared quotation fix resolves it. Unit tests include idempotence, genuine negation, self-repairs, literal bytes, coding/developer modes, uppercase emphasis, mixed interjections and outer questions. The full cleanup/safety integration suite verifies that the prepared quotation remains protected.

## Remaining acceptance gaps

Nine substantive cases remain: `ordinary_paragraph`, `technical_correction`, `email`, `requirements_modifiers`, `meeting_actions`, `two_topics`, `no_wait_request`, `long` and `long_corrected_requirements`. SQLite/Python casing remains imperfect. Ten deterministic outputs are safety fallbacks. Long dictation can remain faithful but insufficiently structured. The 50/50 coarse score does not establish publication-quality writing, universal semantic fidelity or perfect reliability.

Broader validation and setup changes are recorded in [setup recovery verification](../../setup-recovery-2026-10-23.md). Native Windows/macOS and physical Linux installer/device acceptance remain outstanding.
