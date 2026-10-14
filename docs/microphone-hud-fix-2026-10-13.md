# Microphone selection and HUD placement — 13 October 2026

Verified in the development checkout on Ubuntu 26.04.1, KDE Wayland,
kernel 7.0.0-38-generic, and the internal display at 125% scaling.

The HUD's GTK/WebKit window had a 200 logical pixel minimum height despite
the configured 48 pixel height. The capsule rendered at the top of that
transparent window, visibly above the bottom of the display. The Wayland
path now sets explicit size requests on both the webview and GTK window.
GTK permits programmatic shrinking after the assistant response; the
layer-shell compositor still owns window placement and keyboard focus.
This follows the [GTK layer-shell sizing guidance](https://wmww.github.io/gtk-layer-shell/).
Windows and the non-layer-shell positioning path are unchanged by this fix.

Automatic microphone selection now skips input ports marked unavailable,
prefers a connected wired headset jack as well as USB/Bluetooth headsets,
and retains inputs whose active port is null (including the real Bluetooth
source on this machine). Disconnected headset jacks no longer masquerade
as the laptop's internal microphone. Explicit monitor recording remains
supported. The user's saved microphone selection is now `pipewire:@auto`;
all other saved preferences were retained and a settings backup was made.

Verification:

- Native GTK probe: capsule 208 × 48, assistant response 320 × 180, then
  capsule 208 × 48 again. Bottom anchor and disabled keyboard focus retained.
  Desktop screenshots confirmed the capsule at the bottom edge, including
  the saved waveform style. The temporary probe was removed after testing.
- Full default Rust library/integration suite passed after the geometry
  and availability changes. After adding nullable-port handling, the focused
  PipeWire suite passed: 20 passed, 3 hardware tests ignored.
- Rust formatting, all-target Clippy with warnings denied, and diff
  whitespace checks passed on the final code.
- Installed Phonon-2, CPU INT8, transcribed the same labelled English WAV
  twice through the Python sidecar, returning the expected sentence.
  Final decoding took 0.48 and 0.43 seconds; this is a smoke check, not a
  performance qualification. Fixture SHA-256:
  `6bc58a4efdf20daac252b6b1502632601a71efe0308f6757dc1eda34891a7e4f`.
- The installed Phonon-2 also passed 30 recognition/cancellation/recovery
  cycles through the real Rust ASR actor and Python sidecar. The existing
  `rust_actor_transcribes_english_and_recovers_after_cancel` smoke test
  passed using the Phonon model directory instead of the Zipformer directory.
- Actual automatic capture selected SA-TW P10: 2.944 seconds of audio,
  peak 0.6984, RMS 0.0383, no clipping, zero dropped chunks. Test:
  `linux_microphone_delivers_audio_to_capture_pipeline` with
  `REFLOW_SMOKE_MIC=pipewire:@auto`.
- Reflow was restarted with the updated code and the saved Phonon-2 model
  reached loaded/ready state.

The internal HP Victus microphone remains unavailable in Linux. The only
analog input is the disconnected external microphone jack. The repaired
AMD ACP6x machine driver is installed for this kernel, but the kernel log
reports rejection of its signing key and `mokutil --test-key` confirms that
the certificate is not enrolled. This is independent of Phonon recognition.
The existing local repair helper is
`/home/papan/Documents/microphone-repair/finish-enrollment.sh`.
It requires local administrator authentication, followed by Secure Boot MOK
enrollment during reboot. No certificate or security settings were changed
by this app-fix task. The existing repair instructions should be followed
before testing automatic fallback to the internal microphone.

Live shortcut-to-visible-text verification passed with user assistance.
The user confirmed that the pill reacts and text appears. The running app
selected SA-TW P10, captured 7.744 seconds of speech (RMS 0.0769, peak 0.553),
produced a 76-character transcript, and dispatched automatic insertion with
no reported delivery error or manual-copy fallback. Reported release-to-insertion
dispatch was 693 ms; this is one live sample, not a benchmark. KDE reports
Ctrl+Shift+Space as the assigned shortcut, whereas the saved app preference
is Ctrl+Space. The live check used KDE's assigned shortcut.
