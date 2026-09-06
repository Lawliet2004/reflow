# Backend review for 0.2.0

The review covered the Rust recording lifecycle, ASR actor, settings and paired-device persistence, authenticated LAN API, audio decoding, and refinement runtime cleanup. Existing application behavior and the working tree's prior implementation were retained while correcting concrete failure modes.

## Corrections

- Recording starts and stops are serialized. External clients receive a session identity; a stale socket, timer, or stop request cannot terminate a later recording. Audio draining checks session ownership, and automatic completion is delivered to the owning WebSocket.
- The ASR actor ignores cancellation for a superseded session rather than cancelling the current engine stream.
- Settings updates hold one write lock through persistence and publication. Settings and paired devices are written to a temporary sibling, synced, and renamed over the destination. Failed writes do not publish new in-memory settings or device state.
- Pairing codes permit five failed guesses before requiring rotation and are consumed under one lock, including persistence. Concurrent use of one code authorizes only one device. Device names are bounded.
- API audio accepts sample rates from 8,000 to 192,000 Hz. WAV decoding rejects invalid channel counts and non-finite floating-point samples; streaming PCM requires complete 16-bit samples. WebSocket messages and frames are capped at 1 MiB. Each socket retains its resampler phase between audio chunks.
- Refinement orphan cleanup only selects executables inside Reflow's managed binary directory whose parent has disappeared. Tests evaluate selection without terminating real processes.
- Manual injection reports the actual paste result. Delayed clipboard restoration checks that no later Reflow injection or user clipboard change has superseded it before restoring prior text.
- The public health response reports the compiled package version.

## Verification

The Rust library suite passed all 225 tests after these changes. Regression tests cover stale ASR cancellation, competing recording starts, stale timer and client ownership, failed and concurrent settings writes, invalid WAV sample rates, pairing attempt limits, concurrent code consumption, and managed orphan selection. The stale ASR and pairing tests reproduced the defects before their fixes. Existing history, formatting, model selection, API authentication, and runtime tests also passed in that library run.

`cargo clippy --all-targets -- -D warnings` also passed with two build jobs. The final release verification record should additionally include the full integration-test results run against the release tree.

## Limits of this review

Automated tests use mock ASR engines where noted in the test source. They do not prove microphone quality, speech recognition accuracy, GPU admission and inference behavior, or real-device latency across all supported hardware. Native text injection, operating-system hotkeys, clipboard restoration, microphone permissions, and phone-to-desktop networking require platform and device testing. This Windows review does not constitute a live macOS or Linux verification.

The LAN API remains opt-in HTTP on the local network; this review does not add transport encryption. Pairing and bearer authentication protect API access, but traffic confidentiality still depends on the network. Model/runtime downloads, package signing, installed application updates, and long-running hardware inference require their own release checks. Passing tests is evidence for the covered behavior, not a claim of defect-free software.
