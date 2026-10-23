# Procedure formatting and activation hardening

> Historical source checkpoint. See [reported speech and quotation punctuation](../reported-speech/README.md) for the latest pipeline measurements.

Current-source measurements, 23 October 2026. The overall implementation goal remains incomplete. No model weights were downloaded. The model remains the installed Qwen3.5-0.8B Q4_K_M, with the existing llama-server and stable system prompt.

## Current installed-model results

| Context / decoding     | Applied | Coarse contracts | Median / p95 ms | Peak server RSS MiB | Device VRAM MiB |
| ---------------------- | ------: | ---------------: | --------------: | ------------------: | --------------: |
| 2048                   |   40/50 |            50/50 |       270 / 416 |               760.3 |            1238 |
| 4096                   |   40/50 |            50/50 |       272 / 415 |               760.6 |            1264 |
| 8192                   |   40/50 |            50/50 |     270.5 / 419 |               760.4 |            1316 |
| 4096, temperature 0.15 |   39/50 |            49/50 |     272.5 / 419 |               760.5 |            1264 |

Every deterministic context was checked against live `/props`. All 150 input/output pairs, tokens, prompt/decode times, fallbacks and memory observations are retained in `contexts.json`; the 50 low-temperature pairs are in `temperature-015.json`. Ten deterministic outputs are explicitly identified safety fallbacks. The device VRAM measurement includes the resident desktop app; these are observations under desktop load, not isolated performance guarantees. The app’s saved 1024 context preference was preserved. Each temporary benchmark runtime was shut down.

4096 remains the new/unset preference default. Deterministic decoding remains: the latest 0.15 run lost the requirement-list contract and had 11 fallbacks. It improved Python casing in one output but did not produce a reliable overall improvement. Nonzero-temperature runs are stochastic; earlier samples and the unsafe candidate below are retained rather than treating one favorable sample as acceptance.

## Paired output review

At each matching context, 48 of 50 final outputs are byte-identical to the previous `final-contexts.json`. The existing paired review still covers those unchanged outputs. The two changes are complete imperative procedures gaining sentence endings, while inventory fragments and command/literal endings remain protected.

### sequential_steps

**Input**

> first install python then create a virtual environment after that install dependencies and finally start the server

**Before**

```text
1. Install Python
2. Create a virtual environment
3. Install dependencies
4. Start the server
```

**Current**

```text
1. Install python.
2. Create a virtual environment.
3. Install dependencies.
4. Start the server.
```

### sequence_with_constraint

**Input**

> first back up the database then run the migration after that verify the rows and finally restart only if verification passes

**Before**

```text
1. Back up the database
2. Run the migration
3. Verify the rows
4. Restart only if verification passes
```

**Current**

```text
1. Back up the database.
2. Run the migration.
3. Verify the rows.
4. Restart only if verification passes.
```

The conditions and order are retained. `python` remains lowercase in deterministic output; the model still has casing limitations. No new factual additions or meaningful omissions were identified in this bounded review. This is not proof of universal accuracy.

## Safety failure found by actual inference

An earlier temperature-0.15 run accepted this candidate:

> Rahul will finish the frontend by Friday. Priya owns API integration testing on Saturday, and the launch is before Monday.

The dictated source assigned Priya API integration and stated that testing is Saturday. The candidate attached testing and its date to Priya’s integration work. Ordinary content-token matching had ignored the finite verb while permitting a new preposition. `predicate-discovery.json` preserves the historical accepted row and the earlier safety-source hash.

The validator now records finite-copula positions and rejects a newly inserted preposition replacing a removed predicate. The exact observed candidate has a regression test. A faithful split retaining “Testing is on Saturday,” compact “Meeting: Friday” formatting, and genuinely missing grammatical prepositions remain allowed. This is a targeted relationship check, not a general natural-language equivalence proof. Further attachment and discourse cases still need coverage.

The fixture now recognizes both “not MySQL” and “instead of MySQL” as valid exclusion wording. Existing safety tests independently reject reversed database choices. This corrects a stylistic limitation in the coarse contract; no safety threshold was relaxed.

## Discarded prompt experiments

Three compact final-turn instructions were compared on the same 50 inputs through the existing 1024-context server. The proxy changed both chat-template rendering and generation requests; it did not add an inference pass or another model process. The English-specific instruction improved a technical correction and an email boundary. Language-neutral versions regressed the price sentence’s punctuation and did not improve the remaining run-ons. They were not retained for the language-preserving pipeline. `discarded-compact-requests.json` contains all raw outputs and the instructions. Its raw coarse results predate the fixture’s instead-of alternative.

## Remaining quality limits

Ten substantive cases remain: `ordinary_paragraph`, `technical_correction`, `reported_no`, `email`, `requirements_modifiers`, `meeting_actions`, `two_topics`, `no_wait_request`, `long` and `long_corrected_requirements`. SQLite casing in the requirement list and Python casing in the ordered steps also remain imperfect. The cased-input requirement-list diagnostic from the prior review remains unresolved. Long inputs can still fall back to faithful but unstructured deterministic text. The 50/50 deterministic coarse score does not establish publication-quality editing.

## Verification scope

Current full Linux checks: 56 frontend suites / 353 tests; 968 Rust tests passed, 27 ignored; all-target Clippy with `-D warnings`, Rust formatting, frontend TypeScript/lint/format checks and the Vite build pass. Existing CleanupPage effect and bundle-size warnings remain. The new predicate regression is included in the 626 passing library tests. Ignored tests are not passes.

The separate [activation report](../../atomic-activation-2026-10-23.md) covers the shared writer, cancellation boundary, offline dependency pack and frontend ownership checks. Native Windows/macOS acceptance, packaged UI/permission behavior, broader fault coverage and dependency advisories remain open. No release was published.
