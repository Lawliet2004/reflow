# Recording pause verification — 2026-10-24

This extension adds a pause/resume control to local microphone recording while preserving Reflow’s existing manuscript-desk design system.

## Behavior

- Settings → General → **Pause recording button** defaults to enabled. Missing values in older native configurations and frontend settings migrate to enabled; an explicit disabled value persists.
- The optional circular control sits to the left of the floating waveform. Its square pauses capture; play resumes the same session. The cross finishes recording, transcribes the captured audio, and inserts the result.
- Pause discards incoming microphone samples before resampling and voice activity detection. Resume continues the same ASR session and PCM stream. Paused time does not count toward the duration watchdog.
- Both windows consume the authoritative `{ can_pause, paused }` snapshot and `recording:pause-state` event through `useRecordingPause`. A newer event takes precedence over a delayed startup snapshot. Phone/API and meeting/system-mix sessions do not expose pause.
- The main dictation studio provides a keyboard-focusable **Pause recording** / **Resume recording** button for supported microphone sessions. While paused, it says **Recording paused** and holds the waveform, recording ring, and timer still.
- Sessions started by clicking the floating launcher use `start_floating_recording` and require explicit completion. Silence detection and stuck-hotkey limits do not end those sessions, allowing a 30–40 minute active recording. Existing hotkey-session limits remain in place.
- When audio retention is enabled, manually finished floating sessions have capacity for one hour of retained audio. Disabled-retention behavior is unchanged. Native ASR retains its pre-existing one-hour cap.

## Design alignment

The finished extension follows the existing Operate-mode interface: warm paper surfaces, book-face studio headings, restrained sans-serif controls, compact HUD chrome, and existing theme/appearance behavior. The studio action uses the established secondary-button treatment; the settings option uses the existing ruled settings rows and toggle. Paused state is expressed with a short label and stillness.

The recording capsule grows only in width when the pause control is available:

| HUD scale | Pause available | Pause disabled | Native window with pause |
| --------- | --------------- | -------------- | ------------------------ |
| Compact   | 112 × 28 px     | 88 × 28 px     | 120 × 36 px              |
| Standard  | 128 × 32 px     | 100 × 32 px    | 136 × 40 px              |
| Large     | 152 × 36 px     | 120 × 36 px    | 160 × 44 px              |

Native windows add four logical pixels on each side. Control heights remain unchanged.

No visual-system replacement or token changes were required. `PRODUCT.md`, `DESIGN.md`, and the incumbent Impeccable design artifacts were preserved. Context reported no stale-design directive, and no feature-specific visual-system drift was identified in the reviewed evidence.

## Visual evidence

These captures render the actual Reflow components with synthetic audio levels. The temporary component harness was removed after capture.

- [controls-and-settings.png](controls-and-settings.png) — initial recording-control and General-settings evidence.
- [controls-and-studio.png](controls-and-studio.png) — final light/dark recording, paused, and processing controls at compact/standard/large scales, the pause-disabled comparison, General settings, and the main studio’s keyboard route.
- [pause-preview.png](pause-preview.png) — cropped pause-control preview.

Component screenshots and browser/CUA interaction checks establish UI rendering and controls only; they do not exercise native microphone capture.

## Review and checks

- The initial design review required two fixes: gate pause by supported recording source, and provide a keyboard-accessible route in the main studio. Both were resolved. The follow-up verdict was **Pass — ship**.
- The Impeccable detector ran once and returned `[]`.
- Final Rust library suite: **668 passed, 16 ignored**.
- Final full frontend suite: **56 files, 386 tests passed**. The earlier targeted frontend checks passed **21 tests**.
- `npm run build` passed.
- `cargo build --bin reflow` passed; the desktop executable was rebuilt.
- Changed-file Prettier, rustfmt, and diff whitespace checks passed.
- ESLint reported zero errors and the pre-existing warning at `CleanupPage.tsx:108`.

The changed implementation spans `Overlay`, `OverlayApp`, `DictateHome`, `GeneralPage`, the shared `useRecordingPause` hook, frontend/native settings and commands, the event bridge, audio capture, session handling, and native overlay sizing.

## Verification limits

No actual 40-minute microphone session was performed. Native end-to-end transcription and insertion into an external application were not tested in this verification pass. Long-session support is established by implementation and automated checks, with the existing native ASR cap still applicable.
