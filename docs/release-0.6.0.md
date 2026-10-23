# Reflow 0.6.0

**Preview release.** Physical clean-machine acceptance remains pending; see the validation limits below. Reflow 0.5.0 remains the latest stable release.

This release improves first-run setup, offline speech dependencies, shortcut handling, and transcript cleanup. It also includes the accumulated reliability fixes, regression tests, verification records, and promotional video sources and assets since 0.5.0.

## Setup and speech runtimes

- Guided setup checks the microphone, tests the chosen shortcut or Record alternative, recommends a model, installs dependencies and weights, and finishes with a real dictation and delivery trial. Deferred setup can be resumed.
- Managed speech dependencies use pinned Python builds and per-platform, hash-verified runtime packs. Packs install from a local wheelhouse without network access; model weights remain separate downloads.
- Large CUDA packs ship as independently verified parts within GitHub's file-size limit. Reflow resumes and checks each part, then verifies and extracts the archive stream without an extra assembled cache copy.
- Setup and repair recover progress after reopening the UI, reject stale results, preserve the active installation on failure, and support cancellation and cached offline repair.
- Readiness checks verify the selected speech profile and actual imports and operations before reporting success. Intel Macs retain the English Zipformer path.
- Speech runtimes better handle memory constraints, model changes, sidecar cancellation, audio decoding, and device selection.

## Dictation and writing

- Transcript cleanup better handles explicit corrections, negation, repeated emphasis, quoted speech, literal text, and technical replacements.
- Corrections preserve earlier paragraphs and explicit line breaks. Safety validation rejects unsupported changes and retains the deterministic fallback.
- Optional writing setup verifies the runtime before admitting model weights and recovers job state after missed progress events.
- Shortcut capture, gesture handling, desktop integration guidance, setup diagnostics, and latency displays receive reliability improvements.

## Privacy and reliability

- Offline-policy failures keep the download gate closed and report a persistence error with a repair path. Policy changes revoke owned speech processes and reject stale readiness results.
- Shared verified transfers handle interrupted downloads, corrupt caches, cancellation, and installation activation more consistently.
- Release automation builds the dependency-pack matrix before desktop installers and verifies catalog hashes, installer provenance, checksums, and native acceptance evidence before publication.
- Windows lint excludes Linux-only test helpers; socket fixtures explicitly use blocking reads with bounded startup timeouts, and the native-ASR cancellation test no longer races a closed client socket.

## Validation and limitations

Automated checks cover frontend behavior, model-free speech logic, Rust regressions, packaging, and release verifiers. Detailed implementation and installed-model evidence is retained in [implementation-progress.md](implementation-progress.md) and [verification/](verification/).

Local release checks on 23 October 2026 passed: 375 frontend tests, 1,005 Rust tests (28 ignored), 45 Python runtime tests, 24 Python tooling tests, and 20 packaging/release tests (2 platform skips). Production build, TypeScript, formatting, and all-target Clippy passed. ESLint retains one existing non-fatal CleanupPage warning; npm audit reported zero vulnerabilities.

An additional real cached Zipformer pack, split into seven parts, passed offline installation, cancellation, corrupt/missing-cache rejection and repair with zero network journal entries or model downloads; see the [multipart installer result](verification/release-0.6.0/multipart-install.json). This is dependency-installation evidence on the local Linux host.

Physical clean-installer checks on Windows, macOS, and Linux X11/Wayland remain pending. macOS builds retain preview status. Automated builds and synthetic audio do not establish microphone, shortcut, external insertion, signing/quarantine, or GPU compatibility on a physical machine. This preview publishes after automated checks; stable publication still requires the [native acceptance requirements](native-release-acceptance.md).

The installed small writing model still has documented polish and structure limitations. Rust dependency auditing also reports existing advisory warnings, including GLib 0.18.5 unsoundness; see [the advisory findings](verification/dependency-advisories-2026-10-23.md).
