# Linux runtime verification

Verified on 11 October 2026 using Ubuntu 26.04, KDE on Wayland, and Python 3.14 in the checkout's `.venv`.

## Fixes

- Development launches discover `.venv` without shell activation. `REFLOW_PYTHON` explicitly overrides interpreter discovery; an activated environment takes precedence over the checkout environment.
- Missing startup dependencies identify the selected Python interpreter and the installation command, instead of exhausting sidecar crash retries.
- Windows modifier-only settings migrate to a regular Linux shortcut. Wayland uses the GlobalShortcuts portal and exposes desktop shortcut configuration in General settings.
- Wayland automatic paste uses a separate keyboard-only RemoteDesktop permission. Pending or denied access leaves text on the clipboard for manual paste.
- Hugging Face metadata and download timeouts default to 60 seconds. Pinned downloads retry temporary connection failures up to three times while preserving network policy and checksum verification.
- Linux process cleanup recognizes a running executable after Cargo replaces its file, so an old development instance cannot silently block the new build.

## Passed

- A bare development launch selected the checkout's `.venv` and started the Python speech sidecar.
- Zipformer 20M INT8 downloaded, passed pinned file checksum verification, and loaded on CPU.
- Real WAV speech recognition passed through the Python runtime and the Rust ASR actor; actor cancellation also passed.
- Microphone capture reported zero dropped frames. Recognition using a PipeWire speech loopback passed.
- Python runtime suite: 36 tests passed, including transient download recovery. Linux process cleanup: 6 tests passed.
- Frontend suite: 231 tests passed. Complete Rust suite: 803 passed, zero failures, 10 hardware/environment tests ignored in the default run. The three Linux audio/model smoke tests above were run separately and passed. Production frontend build passed.

## Pending

- Phonon-2 installation remains unverified. Earlier attempts hit a read timeout and temporary DNS failures; the final live test hit repeated CDN stream disconnections. A transfer received about 6.2 MB before disconnecting, and the bounded retry correctly started again. Verification was stopped after ten minutes without a completed model. Cached metadata and partial downloads were preserved; no Phonon recognition success is claimed.
- KDE has no active dictation key assigned. Native shortcut activation/release and automatic paste into a focused application remain unconfirmed. Computer use reports no native-app access in this session, so it could not complete KDE configuration or permission dialogs. Assign the dictation key through Settings → General → Configure desktop shortcuts; keyboard access for paste is requested on first insertion.
- These checks establish Zipformer's CPU path. They do not establish that every speech model or GPU configuration works.

## Follow-up verification — 12 October 2026

Additional Linux fixes preserve the existing runtime and desktop-portal work:

- AppImage autostart and generated desktop identities reference the original image, rather than its temporary mount. Desktop launcher paths escape spaces, quotes, backslashes, dollar signs, backticks, and literal percent field codes. Autostart removal reports filesystem failures.
- Shortcut listeners release their owned recordings on session closure, bus disconnection, and task cancellation. Keyboard-session revocation during permission setup prevents a stale ready status.
- Clipboard restoration accepts a confirmed empty Linux selection, while still refusing to erase unsupported formats. Wayland's absent selection is distinguished from missing MIME types or missing protocols.
- Debian packages depend on `xdg-utils` for folder opening and recommend `xdg-desktop-portal`; a compatible desktop backend is still required for Wayland features.

Passed in the final checkout:

- Frontend lint, formatting, type checking, and all 232 tests; production frontend build.
- All 809 default Rust tests, with zero failures and 10 environment-dependent tests ignored; Rust formatting and Clippy with warnings denied.
- All 36 model-free Python regressions and 16 packaging/process-cleanup tests; two Windows-only packaging tests skipped on Linux.
- Real Zipformer 20M INT8 recognition on CPU through the Rust actor and Python sidecar, including cancellation and a second successful dictation using the existing WAV fixture.
- Actual GLib desktop-file validation and launch round trips using the Rust escaping helper and isolated temporary executables, including filenames containing the special characters above.
- Native Linux diagnostics identify Ubuntu and Wayland. The native executable has no missing shared libraries on this machine.
- A debug Debian package builds successfully, contains the desktop launcher, application metadata, speech scripts, and requirements, and declares the updated dependencies. Output: `src-tauri/target/debug/bundle/deb/Reflow_0.4.0_amd64.deb`.

The desktop shortcut assignment, permission dialogs, and automatic paste into a native Wayland application remain pending interactive verification. Phonon and GPU recognition retain the limitations recorded above. This debug package verifies packaging; it is not a published release.

## Performance and polish refactor

See [Linux performance and polish verification](linux-performance-verification.md) for the latest changes, automated and hardware checks, measured limits, local release artifacts, and native acceptance still required. Earlier entries above retain their original verification scope.
