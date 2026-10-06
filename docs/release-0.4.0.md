# Reflow 0.4.0

This release adds the English Zipformer 20M INT8 speech model, automatic dictionary learning from supported Windows text corrections, task-based dictation cleanup, and updated appearance controls. Qwen3-ASR and Phonon-2 remain available. Model weights and Python dependencies require a separate installation; installers do not bundle them.

Model loading now has a 15-minute deadline during status polling. A stuck loader is retired with an actionable error and can be reloaded. Active downloads are excluded from this deadline.

## Backend fixes

- Missing or damaged settings preserve existing encrypted history, transcripts and saved recordings. Startup cannot silently decrypt an encrypted database. Audio with an expired saved deadline is still removed; new audio saving remains disabled during settings recovery.
- Settings created by a newer Reflow version remain untouched. Saving changes is blocked until you update to a version that supports that schema.
- Private local speech and rewriting requests bypass system proxies and refuse redirects. Python model downloads cannot bypass their network policy through environment proxy settings.
- Installing, repairing or rolling back a refinement runtime restores the previously loaded native speech model, including when activation fails.
- Python speech model loads report missing weights and worker failures after the initial loading acknowledgement, allowing the app to show the failure instead of waiting indefinitely.
- Model imports bound the number and total size of metadata files, reject remote-code metadata, and refuse files that change size during import.
- Dictation cleanup explicitly requests a single edited version to discourage repeated prose, lists or summaries of the same text.
- Startup recognizes orphaned servers in verified managed runtime generations, while excluding unrelated servers and live parents.
- A failed audio drain aborts finalization instead of presenting an incomplete transcript as successful. Old canceled audio work cannot update a newer session's levels or metrics.
- New macOS settings use Ctrl+Shift+Space, a shortcut supported by the macOS registry.
- Updated the vulnerable source-map-js dependency and added a Rust dependency security gate to CI.

## Downloads

| Device                 | Download                                                     |
| ---------------------- | ------------------------------------------------------------ |
| Windows PC (64-bit)    | `Reflow_0.4.0_x64-setup.exe` or `Reflow_0.4.0_x64_en-US.msi` |
| Linux PC (64-bit)      | `Reflow_0.4.0_amd64.AppImage` or `Reflow_0.4.0_amd64.deb`    |
| Mac with Apple Silicon | `Reflow_0.4.0_aarch64.dmg` — clipboard preview               |
| Mac with Intel         | `Reflow_0.4.0_x64.dmg` — clipboard preview                   |
| Android 8 or newer     | `Reflow-0.4.0-android.apk` — desktop companion               |

macOS remains a preview: automatic text insertion, active-window integration and startup parity are not implemented. Copy transcripts and paste them manually. DMGs use ad-hoc signing and are not Apple-notarized; macOS may require approval in Privacy & Security. See [Tauri's macOS signing guidance](https://v2.tauri.app/distribute/sign/macos/#ad-hoc-signing).

For your first Mac recording, grant microphone permission, select a microphone, install a speech model and press Ctrl+Shift+Space. Existing saved shortcuts are preserved; if an older installation saved a modifier-only shortcut such as Shift+Win, reselect a shortcut with a key in Settings before recording.

The Android companion records on the phone and sends audio to a paired desktop over certificate-pinned HTTPS/WSS. It requires desktop Settings → Phone and a reachable private network. It does not run speech or rewriting models on the phone.

## Installation and verification

1. Install the download for your operating system.
2. Configure your microphone and hotkey. Install an ASR model, its documented Python dependencies if applicable, and run the recognition check.
3. Choose No LLM for deterministic cleanup, or explicitly install a local rewriting model.

CI tests the release commit on Windows, Linux and macOS, plus the Python runtime and Android companion. Publication waits for every build, native launcher smoke checks, Android signature verification, installer format/inventory checks, matching installer sizes and SHA-256 hashes from each platform's build manifest, and final SHA-256 checksums. Automated checks do not verify every physical microphone, desktop permission, display server, or GPU configuration. See the [audit record](https://github.com/Lawliet2004/reflow/blob/v0.4.0/docs/backend-audit-0.4.0.md) for evidence and limitations.

`SHA256SUMS.txt` covers the uploaded installers and any optional macOS app archives. For rollback, back up your settings and history, then reinstall [Reflow 0.3.0](https://github.com/Lawliet2004/reflow/releases/tag/v0.3.0). Keep the same Android signing key for future companion updates.
