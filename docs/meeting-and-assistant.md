# Meetings, summaries and assistant actions

Enable **Settings → Advanced → Meeting audio**, then use **Home → Record meeting**.
The same Stop and Escape controls end or discard the capture. Reflow mixes the
microphone and playback streams at 16 kHz mono (half of each signal), recognizes
locally, and saves a `meeting` history entry. Meeting output never pastes into
another application. Use headphones to prevent playback being recorded again
through the microphone. Media ducking is skipped for meetings.

Windows uses the default playback endpoint. The installed CPAL 0.15.3 WASAPI
implementation enables `AUDCLNT_STREAMFLAGS_LOOPBACK` when an input stream is
built on a render device. See [Microsoft's loopback documentation](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording).
Playback device changes do not trigger automatic switching during recording.
Protected playback may be unavailable to loopback capture.

On Linux, expose a PipeWire/PulseAudio monitor as a CPAL/ALSA input whose name
contains `monitor`. Inspect your available sources with `pactl list short sources`
and ALSA capture names with `arecord -L`. Distribution ALSA/PulseAudio bridge
configuration determines what CPAL can enumerate. A source visible only in a
PipeWire graph may need an ALSA bridge configuration before it appears here.
When several monitors are exposed, choose one in Advanced settings. Missing or
ambiguous monitors fail explicitly; Reflow does not silently substitute a mic.
X11/Wayland desktop permissions and audio routing differ by distribution. macOS
meeting capture is unsupported with the pinned CPAL backend; file imports remain
available. This implementation was compiled on Windows, not runtime-tested on
Linux or macOS.

Meeting captures stop at one hour, matching the live ASR stream's limit. Audio
retention still follows the bounded buffer and existing retention policy; long
captures may retain the transcript without a complete audio recording. File
imports support two hours through independent timestamped ASR segments.

Open a history entry's menu and choose **Summarize transcript**. Summaries use an
installed local refinement model, split at sentence/paragraph boundaries to fit
its context window (at least 4096 tokens for summary tasks), and join summaries in transcript order. They preserve the
original transcript. An indivisible sentence that cannot fit, missing runtime,
model error, or segment/deadline limit produces an error. The summary can be
copied or explicitly saved as a note. No summary request downloads a model.

The assistant remains stateless. Asking it to search the web or remember a note
can produce one proposal line: `SEARCH: query` or `NOTE: text`. The response HUD
shows **Search web** or **Save note**. Receiving model output performs no action.
Clicking validates the entire proposal again; other commands, multiline
proposals, control characters and oversized content are refused. Search queries
are encoded into a fixed Google search URL; notes enter the local history store.
No shell is executed from assistant output. Browser searches and Check releases
are explicit external-browser actions; the Reflow download ledger does not track
traffic made by that browser.

Speaker diarization is not enabled. The prompt explicitly permits shipping file
transcription with `speakers: null`. The [ONNX community segmentation model](https://huggingface.co/onnx-community/pyannote-segmentation-3.0)
provides local speaker segmentation, but there is no pinned speaker embedding or
clustering pipeline in this repository to establish identities across long
recordings. The [pyannote pipeline](https://github.com/pyannote/pyannote-audio)
requires additional dependencies and pretrained artifacts, with model access
conditions. Adding it would expand installation and offline artifact support.
The nullable segment API and encrypted subtitle storage are ready for a future
validated local pipeline; energy changes alone are not presented as speaker IDs.
