# Reflow 0.7.0 verification

This record accompanies fixes to the 24 October audit. The audit itself describes the pre-fix checkout.

## Regression coverage

- F01: a second SQLite reader prevents encryption before any row changes. Retrying after closing it succeeds; the database, WAL and journal contain no synthetic plaintext marker.
- F02: expired ready meetings lose their audio/transcripts without changing settings; an active session defers cleanup.
- F03: shared launch admission rejects low/nonfinite host memory and accounts for larger context windows without allocating real model weights.
- F04: lint corrections preserve behavior. Compatible lockfile updates remove the reported npm advisories. Existing transitive Rust warnings remain separate.
- F05: cached Python environments require a matching platform/profile/lock receipt and successful live verification. Model setup prepares installed weights as well as new downloads. Frontend tests cover both routes.
- F06: selected/clipboard context is complete, Unicode-safe, and deduplicated; duplicate primary task text is omitted from auxiliary context. A shared prompt/output budget rejects oversized requests with recovery guidance. Cleanup with oversized auxiliary context preserves the transcript. This deliberately avoids lossy automatic truncation; retrieval or summarization of oversized sources is not implemented.
- F07: pinned extracted-file hashes permit read-only reuse without archive access or staging space. Tests cover corruption repair, low disk, cancellation, unsafe tar members, and no-write warm loading.

## Actual archive check

The downloaded archive matched the previously pinned SHA-256. Its extracted model and token hashes are recorded in [omnilingual-integrity.json](release-0.7.0/omnilingual-integrity.json). On this Linux host, preparation took 23.75 seconds cold and 0.21 seconds warm, with extracted file timestamps unchanged on the warm run. These timings cover integrity/extraction, not recognition or model initialization.

## Local checks

Frontend: 409 tests passed across 60 files with one worker. Python runtime: 52 tests passed. Packaging/release: 20 passed, 2 platform skips. Production build, TypeScript, ESLint, Prettier and npm audit passed; npm reported zero vulnerabilities. The Vite main-chunk size warning remains.

The initial concurrent test run encountered memory contention during unrestricted Rust linking; the frontend rerun with one worker passed. One old Rust test asserted silent context truncation and was updated to require complete source preservation. The final full Rust run passed 1,041 tests with 30 ignored and no failures across 46 test-result blocks. All-target Clippy passed with warnings denied. GitHub CI and installer publication are verified separately by the release workflow.

Browser preview inspection covered the Models catalog, offline setup explanation, task filters, unavailable integrations and model details. Preview uses simulated backend state; it does not establish native runtime readiness.

## Remaining acceptance work

Physical clean install, microphone/shortcut/external insertion, GPU, Android device, and five-hour meeting tests are still required. Model quality and performance need real-weight evaluation. Existing GTK3/GLib advisory warnings are documented separately. The release remains a preview; stable acceptance gates were not weakened.
