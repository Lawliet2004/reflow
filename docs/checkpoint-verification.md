# Source checkpoint — 3 October 2026

This checkpoint preserves the native/Python model runtime work, feature expansion, Android companion, UI changes, bug-audit fixes and installer tooling. See [whole-codebase audit](WHOLE_CODEBASE_AUDIT.md), [feature verification](feature-expansion-verification.md) and [packaging validation](packaging-validation.md) for behavior and acceptance limits.

The integrated application checks passed: 177 frontend tests, 595 Rust tests (one existing opt-in hardware test ignored), 30 Android unit tests, Android debug assembly/lint, 17 Python runtime/network-policy tests, 70 Python runtime self-tests, 12 Node packaging/process tests, production frontend build, Rust formatting and all-target Clippy with warnings denied. Windows NSIS and MSI builds succeeded; isolated MSI extraction verified runtime bytes, excluded the development smoke executable, passed the packaged Python self-tests and ran the packaged application help command successfully.

Raw local logs and generated diagnostic JSON referenced by earlier verification records are intentionally excluded from Git because they can contain local machine paths and environment details. The summary records, code, tests, lockfiles, CI, runtime pin manifests and synthetic UI screenshots are preserved. Installers/build output, model weights, application databases, local settings, credentials and signing keys are not part of the source checkpoint.

Rebuilding requires the documented toolchain and dependencies. Models must be downloaded separately. Signing, clean-machine installation/upgrade/uninstallation, physical-device behavior and Linux/macOS acceptance remain separate verification work; local checks do not claim that hosted CI or those physical tests have already passed.
