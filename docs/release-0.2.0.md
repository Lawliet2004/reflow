# Reflow 0.2.0 — A quieter workspace

Reflow 0.2.0 brings a redesigned desktop workspace and fixes reliability problems across dictation, settings, history, and phone pairing.

## A more thoughtful interface

- A focused dictation studio, editable transcript, and one dependable Copy action.
- Restrained light and charcoal themes, clearer typography, responsive navigation, and consistent settings sections.
- Writing preferences that update both cleanup intensity and the processing tier; explicit language choices now disable automatic language detection.
- Searchable, paginated history with accurate date filters and visible failures. Failed deletion keeps the transcript available.
- Vocabulary and replacement rules use the shared settings save path, survive navigation, and retain drafts when saving fails.
- Removed the idle microphone test that never opened a microphone, decorative preview buttons, duplicate title-bar processing controls, and manual insertion buttons that could paste back into Reflow. Hotkey dictation still inserts into the captured target application.

## Reliability and architecture

- Serialized recording lifecycle with session ownership for cancellation, timers, and phone clients.
- Atomic settings and paired-device writes; serialized frontend saves and safe asynchronous event cleanup.
- Single-use pairing codes with bounded guesses; validated audio rates and bounded WebSocket frames.
- Clipboard restoration preserves subsequent copies and newer injections.
- One shared model-status hub replaces competing settings-page polling loops.
- Startup recovery, honest action feedback, and a restrictive production webview content policy.

## Validation and availability

The release verification record is in [release verification](release-verification-0.2.0.md), with backend details in [backend review](backend-review-0.2.0.md). Automated checks establish the tested behavior; they do not certify every microphone, GPU, desktop environment, or Android device. Model weights remain separate downloads. Browser preview supports interface review; dictation requires the desktop application.

For rollback, reinstall the previous release from GitHub. Back up local settings and history before changing versions; this release retains the existing settings/history formats.
