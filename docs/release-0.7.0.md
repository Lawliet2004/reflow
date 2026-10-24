# Reflow 0.7.0 preview

This release brings the expanded model catalog together with fixes from the application audit. It remains a preview while physical clean-installer and long-recording acceptance checks are pending.

## Setup and everyday use

- Speech model selection prepares and verifies the required dependency profile, including when weights are already installed. Verified cached Python generations can be reactivated without downloading dependencies again.
- Models are marked Ready only when loaded. Installed models that need preparation retain an actionable Use control, and setup progress survives closing Settings.
- The Models catalog separates speech, text polish, and meeting notes. It includes Omnilingual 300M INT8, Qwen3.5 4B, MiniCPM5 2B, and Gemma 4 E4B, with explicit runtime requirements and unmeasured-performance labels.
- Meeting notes can use an independent writing model. Unsupported speech integrations are described without offering a broken download action.

## Privacy and reliability

- Meeting encryption checks for active SQLite readers before changing data and removes the plaintext journal before reporting success.
- Meeting retention now runs at startup and periodically while idle. Unfinished recordings remain recoverable.
- Writing-model launch checks available host memory, including summaries and CPU fallback. A rejected launch explains recovery and retains the source transcript.
- Selected text and clipboard context remain complete and are deduplicated. Task requests reserve output space within the shared rendered prompt budget. Oversized context returns an actionable error instead of silently using a truncated selection.
- Omnilingual verifies and reuses intact extracted files; repair checks free disk space, cancellation, and pinned file integrity.
- Rust lint failures are corrected, the formatting draft no longer relies on a state-setting effect, and vulnerable development dependency locks are updated.

## Validation and remaining limits

The release workflow requires frontend, Python, Rust, dependency, packaging, and cross-platform checks before building installers. It verifies runtime packs, installer manifests, checksums, and provenance before publishing.

Physical clean-installer checks on Windows, macOS, Linux X11/Wayland, Android device behavior, GPU compatibility, and a five-hour physical meeting soak remain pending. New model quality and speed are not certified. Existing transitive Rust advisories, including the GTK3/GLib warning, remain documented in the [dependency advisory record](verification/dependency-advisories-2026-10-23.md); passing the default audit gate does not resolve them. Large context requests fail safely; this release does not implement automatic retrieval or lossy context summarization.

See the [original audit](audit-2026-10-24/AUDIT.md) and the [fix verification record](verification/release-0.7.0.md). The original audit describes the pre-fix tree and is retained unchanged as historical evidence. Stable publication still requires the [native acceptance requirements](native-release-acceptance.md).
