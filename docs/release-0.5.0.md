# Reflow 0.5.0

First-run reliability across Linux, macOS and Windows, plus a one-click speech runtime setup.

## First run

- Airplane mode no longer deadlocks setup: onboarding shows a one-time "Allow downloads" consent, the Model page can turn it off inline, and real error text replaces generic download toasts.
- **Set up speech runtime**: one click downloads a pinned, SHA-256-verified standalone Python into Reflow's data folder and installs the speech packages — no system Python or pip needed. NVIDIA Linux/Windows machines get the CUDA torch; other machines get the smaller CPU torch. Cancel, retry, airplane-mode stop, and "Remove speech runtime" (frees disk space) are all supported. Intel Macs get the Zipformer path since PyTorch no longer ships macOS x86 wheels.
- Onboarding now tests the microphone with a live level meter, detects the shortcut being pressed, and offers a "Try it here" box that proves text insertion before finishing. "Set up later" is remembered and can be reopened via "Finish setup".
- Offline model file import is available in onboarding and the Model page.
- Low-memory machines get the 0.6B model recommended; onboarding no longer forces Custom placement.
- Missing Python packages (torch, transformers, …) are detected at startup with clear guidance.

## Microphone (Linux)

- Unavailable sources (e.g. an empty headset jack) are listed as "(unplugged)" instead of hidden, and resolve to a specific error message rather than silently capturing silence or a speaker monitor.
- Saved device gone → fallback to automatic; `pw-record` missing → cpal fallback for the default device only (explicit PipeWire nodes report the missing tool).
- PulseAudio-only servers route to cpal; PipeWire detection uses `pactl -f json info` (locale-safe) and retries failed probes.
- Virtual ALSA PCMs are skipped for capture; `hw:`/`plughw:` hardware stays selectable on ALSA-only systems.

## Shortcuts

- Registration failures surface as a persistent banner (also after startup races).
- Registration is transactional: a failing shortcut restores the previous binding and reverts the whole settings patch.
- Rebinding waits for an active session instead of stranding the recording; unrelated edits no longer rebind.
- Windows: a missed modifier key-up can no longer permanently kill the combo, and hook install waits longer on cold starts.
- Escape is grabbed globally only while recording on non-Windows.

## Text insertion

- macOS paste is implemented (Cmd+V) with a one-time Accessibility permission prompt.
- Wayland clipboard backend is chosen by capability, fixing paste on desktops without data-control.
- An empty macOS pasteboard no longer blocks insertion; file/rich-only clipboard content is preserved instead of cleared.
- X11 adds an `xdotool` paste fallback; pasting into elevated Windows apps reports a clear message.
- Smaller ASR models no longer warm-import torch at startup.

## Models and LLM

- A loaded engine is only "ready" when its model matches the current selection (resolver downgrades still count as ready); switching models, runtimes, or the interpreter reloads safely, queued behind any in-flight operation.
- Incomplete Qwen installs are detected via an install receipt and stay resumable.
- New installs default to deterministic cleanup (no LLM); existing settings are unchanged. Rewrite engine startup is bounded by the deadline so dictation falls back to deterministic text instead of stalling; the missing-LLM prompt fires once per run.
- The runtime download resumes under a stable name, disk space is preflighted, LLM downloads retry, and stale partial files are cleaned up.
- Tray: live status label, working History/Settings/New note/Airplane items; UI updates render on animation frames.
- macOS CI: Developer ID signing + notarization + staple validation when Apple secrets are configured (see README → macOS signing (CI)).
