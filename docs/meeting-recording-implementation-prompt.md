# Reliable meeting recording — researched implementation prompt

Prepared 24 October 2026 against the current Reflow working tree. This is an implementation brief, not a claim that the feature below is already implemented or verified.

## Research and current findings

Reflow is a Tauri 2 / Rust / React application with local ASR, local llama.cpp refinement, SQLite history, and existing meeting capture. Extend those foundations instead of adding a separate web recorder.

- `src-tauri/src/audio/meeting.rs` already mixes microphone and playback audio. Windows uses CPAL/WASAPI loopback; Linux now has a PipeWire path as well as a CPAL monitor fallback. The meeting guide's Linux description is behind the code. macOS currently returns unsupported.
- `src-tauri/src/session.rs` and `src-tauri/src/asr/native.rs` contain one-hour limits. Retained audio uses an in-memory ring buffer. Increasing these constants alone does not make five-hour capture reliable.
- The meeting worker uses `try_send` and counts dropped chunks when the channel fills. Durable recording must not depend on ASR keeping up.
- `src-tauri/src/rewrite/tasks.rs` splits summaries by a conservative byte budget, concatenates outputs, and imposes 256 segments / 180 seconds. This is not a resumable hierarchical summary pipeline.
- ASR unload operations and runtime context-size discovery already exist. Reuse them with explicit resource ownership.
- The working tree already contains unrelated edits, including recording-pause work. Preserve them and inspect the latest state before implementation.

Primary references, checked during research:

1. [Microsoft WASAPI loopback recording](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording): capture playback through a render endpoint; system mix is not automatically restricted to one meeting application.
2. [Microsoft application-loopback sample](https://github.com/microsoft/Windows-classic-samples/blob/main/Samples/ApplicationLoopback/README.md): a possible later process-scoped capture option. Verify OS requirements before exposing it.
3. [Apple ScreenCaptureKit capture sample](https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos): native audio capture and separate microphone output; check API availability and permissions for the supported OS versions.
4. [PipeWire stream documentation](https://docs.pipewire.org/devel/page_streams.html) and [keys](https://docs.pipewire.org/devel/group__pw__keys.html): stream lifecycle and monitor capture; realtime callbacks must avoid blocking work and filesystem I/O.
5. [Qwen3-ASR upstream](https://github.com/QwenLM/Qwen3-ASR): consult the exact installed runtime's inference and segmentation capabilities rather than assuming the whole recording fits one request.
6. [llama.cpp server documentation](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md): context, tokenization, generation limits, and slot behavior. Validate against Reflow's pinned server version, not just upstream main.
7. [SQLite WAL](https://www.sqlite.org/wal.html) and [synchronous settings](https://www.sqlite.org/pragma.html#pragma_synchronous): transaction durability requires an explicit synchronization policy; database transactions do not atomically commit external audio files.
8. [Windows execution-state API](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-setthreadexecutionstate) and [systemd inhibitor locks](https://github.com/systemd/systemd/blob/main/docs/INHIBITOR_LOCKS.md): inhibit automatic idle sleep during recording, while respecting forced sleep and shutdown.

Architectural recommendation: durable native capture first; transcription from saved chunks after Stop by default; optional background transcription later; hierarchical summaries using the selected installed LLM. Unloading ASR frees memory but does not enlarge an LLM's supported context window. Larger contexts are optional optimizations, not prerequisites for long meetings.

## Copy-ready implementation prompt

Implement reliable long-meeting recording, transcription, and summarization in this Reflow repository. Deliver working code, migrations, tests, updated documentation, and honest verification evidence. Do not stop at a plan. Preserve existing uncommitted changes and ordinary dictation behavior.

### Product behavior

Extend the existing Record meeting entry point so I can capture a Zoom, Google Meet, Teams, or other laptop call by recording microphone plus system playback. No meeting bot or provider-specific integration is required. Explain that system playback can include audio from other applications.

Before starting, let me select microphone, playback source where supported, and a maximum duration of 1, 2, or 5 hours. Default to 5 hours. The meeting continues until I press Stop or reach that visible limit. Silence, focus changes, minimization, a locked screen, UI navigation, ASR failures, and LLM failures must not intentionally stop capture. Do not advertise unlimited recording.

Define the limit as elapsed monotonic time from Start, including pauses, with a hard five-hour session ceiling. Show elapsed time, captured duration if different, remaining time, separate mic/system meters, recording state, warnings, and an always-accessible Stop control. If extending the limit while recording, never exceed five hours from Start. Preserve existing pause behavior for ordinary dictation; implement and document meeting pause semantics explicitly if exposed.

Closing the main window should keep recording in the tray. Explicit Quit while recording should offer to keep recording or stop and save. Escape must not silently delete a long meeting. Discard requires an explicit destructive action. Reaching the duration limit must finalize and save normally, then process the recording.

Keep captured audio, transcription status, and summary status independent. Provide retry/resume for processing without re-recording. Meeting output must never auto-paste into another app.

### Audit before changing code

Read applicable repository instructions and inspect `audio/meeting.rs`, `audio/capture.rs`, `audio/pipewire.rs`, `session.rs`, ASR actors/backends, `rewrite/tasks.rs`, `rewrite/server.rs`, history/encryption/retention, app lifecycle/tray handling, IPC, and current meeting UI. Search every capture limit, whole-recording allocation, timeout, silence-stop path, cancellation path, and cleanup task that can affect meetings. Verify current implementation; documentation can be stale.

Reuse suitable components, but give meetings a dedicated durable session/job lifecycle instead of routing a five-hour recording through the short-dictation buffer. Do not merely raise the existing one-hour constant.

### Capture and persistence

1. Own recording in the native backend, independent of React lifecycle and browser timers. Use a dedicated capture supervisor and bounded queues. Audio callbacks must do minimal work: no inference, disk I/O, network calls, or waiting on contended locks.
2. Write audio incrementally on a dedicated writer. Use independently recoverable short chunks, initially around 10 seconds, with a benchmarked durability cadence. Preserve mic and system as synchronized separate tracks where practical, deriving a mix for ASR. Do not keep a growing recording-sized vector or serialize hours of PCM over IPC.
3. Give every chunk a session ID, sequence number, track ID, format, sample count, timeline offsets, checksum, and durable state. Use temporary writes, file synchronization, atomic finalization, and transactional metadata publication in a defined order. Reconcile orphan files and incomplete manifests at startup. Cover directory durability where supported. Avoid a single fragile five-hour WAV whose header must be finalized to recover anything.
4. Use appropriate SQLite WAL/synchronization settings without weakening existing durability. The audio/metadata protocol must survive crashes between any two steps. Keep database writes off realtime callbacks. Periodically checkpoint without long capture-blocking stalls.
5. Define a measurable recovery-point objective: target no more than 10 seconds of captured audio lost on process crash on tested storage. Report the measured result and distinguish process-kill recovery from actual power-loss durability. Previously acknowledged durable chunks must not disappear. Recover an interrupted session on restart; do not pretend audio was captured while the app was down or silently restart recording.
6. Use monotonic timestamps and sample counters, account for separate device clocks, resample correctly, and handle drift. Preserve true timeline offsets across pauses, gaps, reconnections, and silence. Do not concatenate disconnected audio as though time never passed. Test known markers at the beginning, middle, and end.
7. Queue overflow must never silently discard audio. Reserve bounded capture buffers for short writer stalls; shed optional inference work first. If storage cannot keep up, mark exact known loss intervals, surface a persistent degraded state, and preserve all durable chunks. Stop safely with an explicit reason when continued recording is impossible. Never synthesize silence and label it successful capture.
8. Monitor writer health, stream errors, dropped frames, queue depth, disk space, and last successful durable commit. Distinguish legitimate silence/no loopback callbacks from an actual device failure. Device reconnection needs bounded retries/backoff and user-visible status; preserve the unaffected track and avoid tight restart loops.
9. Estimate disk requirements from the actual selected format and number of tracks, with headroom. For scale, 16 kHz mono PCM16 is about 576 MB for five hours per track, excluding overhead; native stereo formats can be much larger. Recheck free space during recording and safely finalize before exhaustion. Ensure retention never deletes active or queued job inputs.

### Platform capture and lifecycle

Windows: use and validate the existing WASAPI loopback path, with microphone captured separately. Handle default playback changes, disconnects, and Bluetooth profile changes. Define whether a selected device stays pinned or follows the default. Do not silently switch a specifically selected device to an unrelated one.

Linux: inspect and extend the existing PipeWire implementation; use real monitor/source discovery and stable identity rather than name matching alone. Validate routing on the available desktop/session, including Wayland where applicable. Keep any PulseAudio/ALSA fallback explicit and capability checked. Never silently substitute microphone-only recording when system capture was requested.

macOS: add an availability-gated native capture backend using supported system-audio APIs, such as ScreenCaptureKit, plus mic capture. Handle microphone/system-audio permissions and permission revocation. Do not request or save video unnecessarily. If platform work cannot be validated here, state exactly what is implemented, compiled, and unverified, and keep unsupported combinations disabled with an actionable explanation.

Expose per-source readiness and meters before recording. Recommend headphones to reduce acoustic double capture; do not claim echo cancellation unless implemented and tested. System-track labels are not speaker diarization, and several remote speakers share the same playback track. Never invent speaker identities.

Hold an appropriate OS idle-sleep inhibitor during capture and release it on every termination/error path. Allow the display to sleep. Handle suspend/resume notifications and mark unavoidable gaps. Do not promise operation through forced sleep, lid closure, shutdown, power loss, or process termination. Block automatic app/runtime restart while recording. Keep a visible recording indicator and a concise reminder to obtain participants' permission where required, without creating recurring approval friction.

### Durable transcription

Default to record first, transcribe after Stop, so capture does not require an ASR model to be loaded or even installed. Use existing supported ASR choices. Make background transcription optional only after durable capture is validated; backlog must live on disk rather than growing memory or blocking recording.

Process saved audio through model-appropriate bounded windows, with silence-aware boundaries and limited overlap where needed. Storage chunks and ASR windows need not have identical sizes. Handle cross-boundary words and overlap deduplication without deleting genuine repetition. Persist ordered timestamped segments incrementally and retain recognition warnings. Do not send five hours to the existing live-stream API or inherit its one-hour limit.

Use durable jobs keyed by session, audio range, model/config identity, and pipeline version. Retries must be idempotent. Persist pending/running/completed/failed work and reclaim interrupted jobs safely on startup. A crash, timeout, OOM, or cancellation must preserve audio and completed transcript segments. Give each job a timeout and bounded retries, but no arbitrary global three-minute budget for an entire meeting. Show real progress from processed audio duration and disclose pending/missing ranges.

### Selected LLM and long-context summaries

Provide a meeting-summary dropdown reusing installed local LLM inventory. Persist the selected model and summary provenance. Do not silently choose another model, download one, or upload data. A different compatible model can be selected explicitly without re-running ASR.

Use explicit resource arbitration. On memory-constrained systems: complete/checkpoint transcription, release ASR model ownership, confirm unloading or terminate its owned worker safely if needed, then load the selected LLM. Avoid repeated model swapping for each chunk. Do not unload a model another active task owns. Respect Keep loaded settings after the job finishes. Budget model weights, KV cache, working memory, and meeting capture overhead. OOM should permit a smaller context, fewer GPU layers, or explicit alternative model/CPU retry without destroying progress.

Unloading ASR frees RAM/VRAM; it does not change the LLM's supported context limit. Determine effective per-request context from model metadata and the running server. Use the correct tokenizer/chat template to count the complete request, reserving output tokens and a safety margin. Verify APIs against the pinned runtime. Never assume character count equals token count or force an unsupported context size.

Implement a resumable hierarchical map/reduce pipeline:

- Partition the full transcript into timestamped, token-budgeted spans, preferably at semantic boundaries. Fall back to token boundaries for huge sentences, unpunctuated ASR, and multilingual text; do not fail merely because one sentence is too long.
- Extract structured facts per span: topics, decisions, action items, explicitly stated owners/deadlines, unresolved questions, and source segment IDs/timestamps. Preserve disagreements and uncertainty; missing owners/deadlines stay unspecified.
- Persist intermediate results and recursively merge bounded groups until a final overview fits. Carry fact/source references through reductions, preserve distinct actions and decisions, and deduplicate without collapsing conflicting claims. Require progress/shrinkage and bounded retry behavior if a reducer fails to reduce.
- Save detailed action/decision records separately so a short overview cannot erase important items. Produce readable summary sections and source links to the transcript. Validate source references and maintain an explicit coverage manifest of all input spans; indicate any unprocessed ranges and label partial output.
- Never achieve fitting by silently dropping the middle/end of the transcript. Treat transcript content as untrusted data, never instructions. Do not run tools or external actions based on meeting speech.
- Persist processing provenance, stage progress, errors, cancellation, and restart checkpoints. Cache keys must include transcript revision, model/config, prompt/pipeline version, and source ranges; transcript edits invalidate affected summaries. A summary failure never invalidates a completed transcript.

Do not make a very large-context LLM a requirement. Demonstrate five-hour transcript processing with a modest supported context, for example 4K or 8K, as well as a larger context when available. Claim factual quality only to the extent measured; hierarchical summarization can still omit or distort details, so keep the original transcript accessible.

### Privacy, storage, and interface

Keep processing local under the existing privacy settings. Reuse encryption and retention correctly for raw chunks, manifests containing content, transcripts, and summary intermediates. If encryption is enabled, do not spill plaintext audio temporarily; use authenticated per-chunk encryption with unique nonces and crash-safe key handling. Fail visibly if required keys are unavailable. Temporary durable audio is needed even when long-term audio retention is disabled: explain that policy, retain unfinished job inputs, and remove them after successful processing according to an explicit recoverable-failure policy. Deleting a meeting must cancel its jobs and remove associated content without racing active writers.

Use the existing UI language/design system. Distinguish Recording, Saving, Transcribing, Summarizing, Ready, Interrupted, and Failed; model capture and processing states separately internally. Offer full transcript, summary, retry, copy/export, and source navigation. Preserve existing export formats and accurate subtitle offsets. Make controls keyboard accessible. Prevent conflicting dictation sessions from taking over meeting audio resources.

### Verification and acceptance

Add meaningful unit/integration tests for lifecycle transitions, idempotent Stop, stop/limit races, clock behavior, chunk recovery, gap accounting, overflow, disk exhaustion, device changes, job resume, retention/encryption, and model ownership. Test token budgets with multilingual/unpunctuated input, oversized segments, recursive reduction, conflicting facts, prompt injection, and critical facts near the end of a long transcript.

Run accelerated duration-limit tests for 1/2/5 hours, plus a real wall-clock five-hour capture soak on available supported hardware. An accelerated simulated stream is not a substitute for this soak. Record environment, audio route, format, memory/queue bounds, CPU usage, disk growth, expected versus saved sample counts, drops/gaps, and recovery behavior. Include known timed audio markers and periods of silence. Test minimized/locked UI, frontend reload, intentional ASR/LLM worker crashes, slow inference, temporary device loss, and stop during processing. Fault-inject process termination during chunk writes and metadata commits and recover the session. Separate destructive disk/power simulations from tests on actual user data.

Verify that capture remains healthy while inference fails, memory does not grow with recording duration after warmup, late meeting content reaches both transcript and summary, and missing ranges are never called complete. Document a measured RAM budget based on chosen buffers, not an unsupported blanket claim. Run existing relevant frontend/Rust checks and regression tests for dictation, pause, history, encryption, and cleanup.

Do not label the feature production-ready for five-hour meetings until the real soak passes. If hardware or platform availability prevents a check, supply the runnable harness and report the precise remaining validation gap. Do not invent successful results.

Deliver the implementation, any additive migrations, updated meeting documentation, a platform capability matrix, and a concise verification report. Prioritize durable capture and recovery, then reliable transcription, then summaries, then polish. Keep the scope focused on dependable meeting capture rather than adding bots, automatic meeting discovery, calendar integration, or unvalidated diarization.
