# Durable meeting recording

This implementation is experimental. A real wall-clock five-hour hardware soak
has **not passed**; selecting five hours is a deadline, not a reliability claim.

## Use

Enable Settings → Advanced → Meeting audio. The Meeting recordings strip is
available above every main page. Expand it, choose microphone, playback source,
1/2/5 hours (default 5), and an installed local summary model or Transcript only.
Test sources opens both native streams for three seconds; speak and play audio
while testing. Quiet playback meters can mean legitimate silence. Headphones
reduce acoustic double capture; echo cancellation is not implemented. Obtain
participants' permission where required. All playback through the chosen source,
including other apps, is captured. Track names do not identify individual speakers.

Capture does not require ASR or an LLM. Stop and save remains in the strip and
tray. Close hides the main window to the tray. Quit during capture opens a choice
to keep recording or stop/save/quit. Escape does not discard a meeting. Deleting
requires explicit confirmation; active capture/processing must be stopped first.
Meeting output never enters the dictation injection path.

Meetings cannot be paused. The monotonic deadline runs from native Start,
including startup and any device gaps, with a hard five-hour ceiling. There is no
in-session extension control. UI navigation, reload, silence, and inference
failures do not own capture. Automatic idle sleep is inhibited (display sleep is
allowed). Forced sleep, lid closure, shutdown and power loss are not supported
continuity guarantees. Callback gaps are recorded when observed; explicit OS suspend/resume notifications remain unimplemented. The app does not resume
capture automatically after a process restart.

## Storage and crash protocol

`<app-data>/meetings/meetings.db` is a separate additive schema (version 1).
SQLite stores each track as independently addressable 16 kHz mono PCM16 chunks,
about two seconds each. The transaction contains session UUID, track, sequence,
start sample, frame count, SHA-256, audio bytes and updated durable counters.
SQLite WAL with `synchronous=FULL` owns atomic publication and synchronization;
there is no separate external audio file to rename or reconcile. Never copy just
the database while it has a live WAL. Directory synchronization is requested on
Unix. Passive automatic WAL checkpoints run at 256 pages. PCM, transcripts,
summary nodes, job states and session metadata use existing AES-256-GCM envelope
code with fresh random nonces and per-session/field associated data when history
encryption is enabled. Keys remain in the OS history credential store. Missing
keys fail closed. Privacy changes migrate the meeting database while idle.

The writer commits every two seconds and on Stop. The recovery-point objective
is at most ten seconds on tested storage. Recovery marks an active session
Interrupted and running processing Interrupted; checksum verification happens
before decoding any chunk. No down-time audio is invented. A process-kill result
is not a power-loss durability result. Unsynchronized in-flight samples can be
lost; previously committed chunks are protected by the SQLite transaction.

Callbacks convert bounded packets and use a nonblocking queue (128 × at most
8192 f32 samples, about 4 MiB payload worst case). The writer resamples and holds
two 32,000-frame buffers (about 250 KiB), independent of meeting length. This is
a capture buffer budget, not total app RSS: native libraries, child processes,
SQLite pages, inference models and allocator overhead add memory. Processing
reads at most a 30-second mono ASR window plus one short storage chunk. Metadata
and transcript text grow with content; raw PCM does not accumulate in memory.
Two five-hour PCM16 tracks use roughly 1.152 GB decimal; admission reserves twice
that plus 128 MiB for WAL/headroom. Free space is checked every five seconds;
64 MiB remaining triggers safe finalization. Overflow stops capture visibly,
preserves committed input, and never declares dropped audio successful.

Devices are pinned at Start. Linux resolves the selected source against the
native PipeWire graph and targets its object serial with fallback disabled;
CPAL explicit selections fail instead of falling back to an unrelated device.
Each failed track has up to three reconnection attempts with 1/2/4 second
backoff while the other track continues. Known disconnections and microphone
callback gaps are timestamped. Playback callback inactivity alone is not called
a device failure. Persistent failures safely stop/save the session. Physical
Bluetooth/profile/default-change behavior still needs the platform tests below.
The resampler carries sample phase; sample cursors absorb callback jitter.
Clock excursions over 250 ms preserve an offset rather than joining samples
across the gap. Fine long-run independent-clock drift correction is not yet
measured; this is one reason five-hour readiness is unclaimed.

Finished audio follows the audio retention setting, including 1/7/30 days and
forever. Disabled retention still writes temporary durable input; it is removed
only after successful requested processing. Failed, cancelled, interrupted and
pending jobs retain recoverable inputs until explicit deletion. Completed meeting
text follows history retention. Retention runs at startup, on policy changes and
after successful processing. Deletion is serialized with resource ownership.

## Transcription and summaries

After normal Stop or deadline, the app starts processing if a speech model is
loaded. Otherwise the saved session says to load a model and Retry processing.
Track-contiguous chunks form windows of at most 30 seconds, preferring the
quietest 100 ms in the last five seconds. Gaps preserve timeline offsets. There
is no overlap text deduplication, so genuine repetition cannot be removed by a
string heuristic; continuous speech can still suffer boundary recognition
errors and needs quality evaluation. Each window has an audio-range/model/config/
pipeline cache key, a 120-second inference deadline and at most three attempts.
Completed segment data survives cancellation and restart. Retrying a completed
transcript for a different summary does not run ASR again. TXT, Markdown, SRT
and VTT exports keep session-relative offsets; raw playback and speaker
identification are not supplied by this UI.

The shared session admission lock owns capture/processing resources and prevents
competing dictation, file jobs, model reloads and runtime activation. ASR unload
is acknowledged before launching the selected summary LLM once for the job.
Previous ASR ownership is restored according to Keep loaded; refinement respects
Keep warm. Existing runtime memory admission and CPU fallback are reused.
If a selected model/runtime cannot load, progress remains resumable. No alternate
model is selected, downloaded or contacted automatically.

Summary requests require the pinned runtime's `/apply-template` and `/tokenize`;
there is no character-to-token fallback. The effective server context reserves
768 output tokens and a 128-token safety margin. Unicode boundaries provide a
fallback split even for huge unpunctuated or multilingual segments. Map nodes
extract typed facts with validated segment references. Bounded reductions merge
nodes recursively, check shrinkage, and stop with checkpoints if reduction makes
no progress. The full map fact ledger remains separate from the short overview.
A coverage manifest stores input spans, transcript revision, selected model,
context and pipeline version. All maps and reductions have idempotent keys.
Coverage measures processed input, not factual accuracy. Facts can still be
omitted or distorted by a local LLM; keep the transcript available. Meeting
speech is untrusted data and never dispatches assistant tools.

## Platform matrix

| Platform                             | Implemented path                                                                                     | Verification in this change                                                                                                         |
| ------------------------------------ | ---------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| Linux / PipeWire                     | `pactl` source discovery + native graph serial + `pw-record`, separate tracks, login1 idle inhibitor | Linux compilation/unit/integration tests; short native route and crash harness. Physical mic routing on this host failed readiness. |
| Windows                              | CPAL WASAPI render-endpoint loopback + separate CPAL mic, pinned names, execution-state inhibitor    | Source implementation only; Windows compile/device validation unavailable here.                                                     |
| macOS                                | Disabled with an explanation and audio-import alternative                                            | ScreenCaptureKit backend and permission handling are **not implemented**. No macOS compilation/runtime claim.                       |
| Linux without PipeWire tools/monitor | Meeting capture disabled with an explanation                                                         | No silent microphone-only substitution; no new ALSA/Pulse fallback claim.                                                           |

## Remaining acceptance work

Run `python3 scripts/meeting-soak.py --seconds 18000 --mic <id> --playback <id>
--report <path>` on a supported physical route. The harness creates an isolated
temporary database, saves telemetry and checks all checksums/sample counts. Use
`--kill-after 7.3` for process-kill recovery; this does not simulate power loss.
It preserves test inputs under the printed temporary directory for inspection.
The two ignored native Rust tests are its workers.

Still required: real five-hour hardware soak with timed speech markers, physical
device and Bluetooth changes, forced suspend/resume, frontend reload/locked
screen/tray interactions in the packaged app, actual disk-full/slow-storage fault
injection, encrypted OS-key-store crash testing, Windows compilation and hardware
tests, fine drift measurement, macOS implementation, and measured ASR/summary
quality (including boundary words and factual recall). Synthetic five-hour
transcripts and virtual PipeWire routes do not satisfy the physical soak.
