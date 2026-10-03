# Browser verification — 1 October 2026

Inspected the running Vite preview through the in-app browser. Speech showed all canonical languages plus Auto-detect, default-device/microphone controls, health/recognition actions and recovery states. Performance showed presets, runtime plan/inventory and the labelled calibration form.

At 640 × 480, `clientWidth = scrollWidth = 640`; calibration controls fit without horizontal overflow. Run was disabled for a blank reference. Navigation and form focus/scroll were exercised. Console entries were empty. The viewport was reset afterward.

This verifies preview rendering/interaction with fallback native API data. It does not establish microphone/native IPC/clipboard/physical-phone/screen-reader behavior. Those limits are in the full audit.
