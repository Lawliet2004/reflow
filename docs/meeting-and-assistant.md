# Meetings, summaries and assistant actions

Enable **Settings → Advanced → Meeting audio**, then expand **Meeting recordings**
above any main page. Choose microphone, playback source, a 1/2/5 hour limit
(default five), and an installed summary model if wanted. Audio is saved first;
transcription and hierarchical summaries run after Stop. No speech model is
needed to record. Stop is also available in the tray. Escape does not discard
meetings. Devices stay pinned and playback can include other applications.

See [Durable meeting recording](meeting-recording-architecture.md) for storage,
encryption/retention, recovery, platform capabilities, harnesses and exact limits.
The feature is experimental: the required five-hour hardware soak has not passed.
macOS system-audio recording remains disabled; ScreenCaptureKit is not implemented.

The ordinary **Summarize transcript** history action retains its existing short
summary pipeline. Durable meetings use their own persisted summary jobs and
export/source controls. They do not auto-paste into another app.

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
