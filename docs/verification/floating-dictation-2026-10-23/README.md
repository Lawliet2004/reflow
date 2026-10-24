# Floating dictation control

Implemented 2026-10-23. Settings → General → Floating dictation button enables or disables the launcher immediately and persists the choice across restarts. Enabled by default, including for older settings files.

The microphone accepts pointer clicks without native keyboard focus. Starting uses the existing Dictate session pipeline and remembers the external insertion target. The pill shows the live waveform; its circular cross **stops and transcribes**, rather than cancelling. Processing and insertion use the existing output settings and fallback feedback. Once complete, the launcher returns. Empty dictations return immediately without displaying a no-speech message in the pill. Turning the launcher off retains the existing shortcut-driven HUD. Assistant answers keep their window geometry when this preference changes.

Following the user's screenshot feedback, the pill is narrower horizontally while retaining its previous height. Floating sizes in CSS pixels are:

| Size     | Pill     | Idle button | Native recording window |
| -------- | -------- | ----------- | ----------------------- |
| Compact  | 88 × 28  | 28 × 28     | 96 × 36                 |
| Standard | 100 × 32 | 32 × 32     | 108 × 40                |
| Large    | 120 × 36 | 36 × 36     | 128 × 44                |

The native window adds four pixels on each edge for the existing shadow. Legacy non-interactive HUD sizes are unchanged.

## Verification

- Production TypeScript/Vite build.
- Frontend tests covering launch → record → stop → process → idle, duplicate stop clicks, failed-start retry, disabling the control, settings changes, and stale assistant answer cleanup.
- Rust overlay tests covering insertion target preservation, stable legacy geometry, and small floating/idle geometry.
- Rust settings tests covering legacy migration and persistence of the disabled preference.
- ESLint: no errors; existing CleanupPage effect warning remains outside this change.
- Prettier, rustfmt, and diff whitespace checks.
- Impeccable mechanical detector: no findings on changed UI.
- Browser inspection: actual Overlay component in light/dark at all three sizes and recording/processing/completion states, captured in `states.png` before the follow-up reduction to the dimensions above. Idle launcher also inspected in the browser. Existing DESIGN.md palette, type, and reduced-motion rules remain in use; no design-system replacement.

The state gallery used synthetic audio levels and completion text; its temporary harness was removed after capture. Native end-to-end microphone capture, window focus, and insertion across external applications were not exercised by these browser/component checks. These continue to depend on the existing platform dictation support and permissions.
