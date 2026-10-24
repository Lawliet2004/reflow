# Meeting verification — 24 October 2026

Experimental implementation; **not production-ready for five-hour meetings**.

## Checked

- Linux `cargo test --lib -- --test-threads=4`: **682 passed, 18 ignored**.
- Frontend Vitest suite: **391 passed across 57 files**. Final meeting-control rerun: 5 passed.
- `npm run build`, TypeScript, ESLint, Rust formatting and whitespace checks pass. ESLint retains one existing `CleanupPage.tsx` effect warning; Vite reports the existing large-bundle warning.
- Browser setup reviewed at 1280px and 667px widths. Existing paper/ink tokens retained. Mechanical design detector returned no findings. Native locked-screen/tray behavior is not established by a browser preview.
- Unit/integration coverage includes 1/2/5-hour boundary clocks, concurrent/idempotent Stop, dictation resource admission, encrypted audio/work authentication and migration, transactional disk-full rollback, overflow stop, crash recovery, retention protection, gap-preserving ASR windows and beginning/middle/end sample markers.
- Synthetic five-hour transcript: 600 timestamped spans, huge unpunctuated multilingual input, critical final action; deterministic fixture server at 4K and 8K context. Recursive coverage and restart cache reuse pass. This proves pipeline mechanics, **not real-model factual recall**.
- Installed Qwen3.5 0.8B Q4_K_M with llama.cpp **b11146 CPU**, 4K context: `/apply-template` + `/tokenize` counted **94 prompt tokens**, matching completion usage exactly. A single action/owner/deadline fixture with an injected command produced the expected referenced JSON without tool execution. See [protocol result](local-llm-protocol.json).

## Native route and process crash

Host: Linux, PipeWire 1.6.2 / Pulse compatibility server. Selected hardware playback monitor plus a temporary `module-remap-source` test input; both tracks receive the selected playback route. **This is a virtual microphone route, not a physical microphone soak.** The temporary source was unloaded afterward. Native capture needs no ASR model; the isolated test context uses mock recognition and retains its audio for checksum checks.

- [Twenty-second run](native-virtual-route-20s.json): 20.935 seconds from native start (20 seconds after start acknowledgement), saved samples 330,752 / 323,584, peak queue **23 / 128 packets**, final queue 0, no reported gaps/overflows, every PCM checksum valid. The two-second commit cadence is exercised by real native streaming.
- Test-process RSS observed **69.82–72.19 MiB**; main test-process CPU consumed about **0.23 CPU seconds** over the observation interval. Child-process memory/CPU is not included. Twenty seconds cannot establish five-hour memory stability.
- Database/WAL growth at final observation: **1,804,320 bytes**; raw payload is 16 kHz mono PCM16 per track. Capture payload buffer bound is about 4 MiB queue + 250 KiB track buffers, separate from total process memory.
- [SIGKILL at 7.3 seconds after acknowledgement](process-kill.json): SQLite integrity OK, every recovered chunk checksum valid, all previously observed committed chunks preserved. Conservative process-start-to-kill recovery-point bound **1.864 seconds**, below the 10-second target on this storage. Reopened session marked Interrupted. This is **process-kill recovery**, not a power-loss experiment.

The host's default digital microphone node exists but `pw-record` fails to link it (`defined target not found`). The stereo input is flagged unavailable by source discovery. Missing/unusable selections fail visibly; the app does not choose an unrelated microphone. Physical mic recording therefore remains unverified here.

## Outstanding acceptance

The real wall-clock five-hour physical hardware soak has **not run**. Use [the harness](../../../scripts/meeting-soak.py) with `--seconds 18000` on a working physical route. It preserves isolated inputs and telemetry. Virtual routes and accelerated/synthetic fixtures are not substitutes.

Still open: Windows compilation/hardware tests; ScreenCaptureKit/macOS implementation (disabled); fine clock drift; physical Bluetooth/default-device/disconnection recovery; native minimize/lock/frontend reload/Quit flow; forced suspend/resume; actual slow-disk/full-disk/power-loss faults; OS-key-store recovery; real-ASR boundary word quality and measured long-meeting factual recall. See [architecture and platform matrix](../../meeting-recording-architecture.md).

Existing unrelated dictation/floating/pause work was preserved. Regression failures found during implementation were corrected before the final passing run. No production-ready claim is made.
