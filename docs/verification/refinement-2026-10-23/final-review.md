# Current-source transcription verification

> **Latest source:** See [procedure and activation hardening](hardening/README.md). The measurements below identify an earlier source revision.

Measured 23 October 2026 on the existing Kubuntu host with the installed Qwen3.5-0.8B Q4_K_M. No weights were downloaded. This is a text-pipeline benchmark, not microphone/ASR or clean-installer acceptance. The implementation goal remains incomplete.

The final runs query live `/props` and match the actual per-slot context to the client budget. New/unset preferences retain a 4096 default; the desktop app’s saved 1024 preference was preserved. Each context uses one owned temporary benchmark runtime, which is shut down. The already-running desktop app remains resident; VRAM is device-wide and timings are observations under desktop load.

| Run                    | Applied | Coarse contracts | Median / p95 ms | Peak server RSS MiB | Device VRAM MiB |
| ---------------------- | ------: | ---------------: | --------------: | ------------------: | --------------: |
| 2048                   |   40/50 |            50/50 |     271.5 / 400 |               761.5 |            1238 |
| 4096                   |   40/50 |            50/50 |       271 / 416 |               761.2 |            1264 |
| 8192                   |   40/50 |            50/50 |     289.5 / 451 |               761.3 |            1316 |
| 4096, temperature 0.15 |   40/50 |            50/50 |       271 / 409 |               761.0 |            1264 |

4096 remains a reasonable memory/quality default: 8192 added VRAM with equivalent reviewed outputs. Temperature 0.15 changed bullet markers and two comma choices; it did not resolve the outstanding quality problems or reduce fallbacks, and its median matched deterministic decoding. Temperature 0 remains. These measurements do not establish a causal latency improvement from the cached normalizer; the earlier runs had changing desktop/compile load.

The original 4096 baseline applied 25/50 and met 37/50 coarse contracts. The intermediate implementation applied 40/50 and met 49/50. The current request fixes the remaining lowercase-source requirement-list contract, bringing the fixed 50-case set to 50/50. A coarse pass is not publication-quality acceptance. Ten final outputs are explicitly reported safety fallbacks, not successful rewrites.

## What changed

- A compact final user instruction handles the existing conservative parallel-requirement case. Ambiguous purpose/relative clauses, custom formatting and minimal cleanup do not receive this instruction. The stable system prefix and factual safety gates remain unchanged.
- Calendar-name normalization uses one cached regex rather than compiling a regex for each name on every call. ASCII case matching avoids a Unicode long-s lookup panic; quoted text, code and embedded identifiers remain protected. URI schemes are now also treated as identifiers.
- The opt-in measurement tool can reuse an explicitly provided loopback server without spawning another model process. The final three-context measurements still use owned runtimes because they must measure different actual context sizes.

## Paired review

Compared with the earlier deterministic `contexts.json`, **49 of 50 outputs are byte-identical at each matching context**. The original [50-case paired review](README.md#case-by-case-comparison) therefore remains applicable to those outputs. The one changed output follows; all 150 current input/output pairs are also in `final-contexts.json`.

### independent_requirements

**Input**

> the app should work offline use sqlite for storage support dark mode and keep ram usage low

**Current output**

```text
- The app should work offline.
- Use sqlite for storage.
- Support dark mode.
- Keep RAM usage low.
```

Applied rewrite: yes.

### tea_coffee

**Input**

> I want tea no no sorry I want coffee

**Current output**

```text
I want coffee.
```

Applied rewrite: yes.

### ordinary_paragraph

**Input**

> hey I wanted to ask you about the project I finished the frontend yesterday and I'm working on the backend today

**Current output**

```text
Hey, I wanted to ask you about the project I finished the frontend yesterday and I'm working on the backend today.
```

Applied rewrite: yes.

### technical_correction

**Input**

> use react no actually svelte and connect it to postgres not mysql

**Current output**

```text
Use svelte and connect it to postgres not mysql.
```

Applied rewrite: no — safety fallback.

The list now separates all four requirements and preserves their words, but `sqlite` remains lowercase. The ordinary paragraph still lacks a sentence boundary, and the technical correction safely falls back with imperfect proper-name casing. These outputs demonstrate why 50/50 coarse contracts is insufficient.

## Remaining demonstrated quality gaps

The original review still identifies 12 substantive polish/structure cases: `ordinary_paragraph`, `sequential_steps`, `technical_correction`, `reported_no`, `email`, `requirements_modifiers`, `meeting_actions`, `two_topics`, `no_wait_request`, `sequence_with_constraint`, `long` and `long_corrected_requirements`. The changed requirement-list case retains the SQLite casing issue. Long inputs can still return faithful but unstructured deterministic text.

An additional cased-input probe (`The app should work offline use SQLite for storage support dark mode and keep ram usage low.`) produced a candidate containing `Use SQLite for storage support` before a second `Support dark mode` bullet. The unchanged safety gate rejects that relationship change. A database-casing preprocessing experiment exposed this failure and was reverted; casing must not be treated as proof that the model will format a clause correctly. The raw `uppercase-requirements-probe.json` is diagnostic output without the production gate, not an accepted rewrite or an extra case in the 50-case score.

No new factual addition or meaningful omission was identified in this bounded final-output review. This does not establish universal fidelity. No validator thresholds were relaxed to improve the score. Shorter general prompt probes are retained as discarded diagnostics; they did not fix the main run-on/list/reporting problems.

## Evidence

- `final-contexts.json`: 150 current production-pipeline outputs with live context checks, timings, token counts and memory observations.
- `final-temperature-015.json`: 50 paired current low-temperature outputs.
- `final-summary.json` and `final-provenance.json`: metrics, source/model hashes and scope.
- Earlier `baseline.json`, `contexts.json`, `temperature-015.json` and `provenance.json` remain unchanged historical evidence. They do not identify the current source revision.
