# Release verification — 0.2.0

## Scope

The existing working tree contained a substantial unreleased implementation. This release preserves that implementation, repairs the build, reviews backend boundaries, and redesigns the application workspace. Local agent configuration, generated knowledge graphs, scratch utilities, and private diagnostics are excluded from the release.

## Acceptance checks

- TypeScript build, ESLint, Prettier, and component/unit regression tests.
- Rust library and integration tests, Rustfmt, and all-target Clippy with warnings denied.
- Python speech-runtime self-test and frontend dependency audit.
- Real-browser walkthrough of Home, History, settings search, theme changes, dictionary editing, and transcript copying at the default 1040×780 and minimum 640×480 desktop sizes.
- Windows NSIS/MSI packaging, followed by GitHub release build verification.

## Architecture

Local validation passed 67 frontend tests, 354 Rust tests, and 60 Python runtime self-test checks. The installed speech-recognition and refinement integration tests ran on the Windows development machine. The frontend dependency audit reported zero vulnerabilities. ESLint passed with six warning-level React effect findings; TypeScript, Prettier, Rustfmt, and Windows Clippy passed. Linux CI caught a Windows-only GPU-parser import, which is now conditionally compiled. Installer verification caught and corrected the NSIS language identifier before publication.

Rust owns recording state, model execution, persistence, and OS integration. React sends settings patches through a serialized queue and displays authoritative results; failed writes reload persisted settings. Model/cleanup pages consume a shared status hub. Asynchronous event registrations have explicit ownership and release subscriptions even when setup finishes after unmount.

Copy is the manual transcript action. Automatic insertion belongs to the hotkey recording flow, where an external target window was captured before recording. This avoids claiming that a hub button can reliably paste into another foreground application.

The production webview allows bundled code, local IPC, and local/data images. Remote scripts and frames are blocked. Configuration follows [Tauri's CSP guidance](https://v2.tauri.app/security/csp/).

## Limits

The local test machine is Windows. Installed ASR and refinement runtime tests ran here; some hardware-conditional tests skip their live body when the required models/runtime are absent. No universal GPU, microphone, Android-device, macOS runtime, or Linux display-server certification is claimed. Installers are unsigned unless signing is configured in the repository workflow. The optional phone API uses HTTP on a trusted local network.
