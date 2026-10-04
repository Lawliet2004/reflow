# Reflow 0.3.0

Choose Qwen3.5 0.8B, Qwen3.5 2B, or **No LLM** on Home and during first-run setup. The LLM cards in Settings → Performance now select your model. Changing cleanup intensity preserves that choice. Choosing a model does not download it; installation remains a separate action.

## Downloads

| Device                                      | Download                                       |
| ------------------------------------------- | ---------------------------------------------- |
| Windows PC (64-bit)                         | `.exe` installer or `.msi`                     |
| Linux PC (64-bit)                           | `.AppImage` or `.deb`                          |
| MacBook / Mac with Apple Silicon (M-series) | `aarch64.dmg` — macOS preview                  |
| MacBook / Mac with Intel                    | `x64.dmg` — macOS preview                      |
| Android phone (Android 8 or newer)          | `Reflow-0.3.0-android.apk` — desktop companion |

macOS is a preview: automatic text insertion, active-window integration and startup parity are not implemented. Use the app's Copy action and paste your transcript manually. The DMGs have ad-hoc signatures and are not Apple-notarized; macOS can require approval in Privacy & Security. See [Tauri's signing guidance](https://v2.tauri.app/distribute/sign/macos/#ad-hoc-signing).

Android records audio and sends it to your paired Reflow desktop over HTTPS/WSS. It does not run speech or LLM models on the phone. In desktop Settings → Phone, enable the LAN API and pair the phone using the certificate-pinned pairing link. The phone and desktop must be reachable on the same private network. This APK is for direct installation, not a Play Store listing. There is no iPhone/iPad app in this release.

## Getting started

1. Install the desktop download for your operating system.
2. Choose your microphone and hotkey, then install an ASR speech model and run the recognition check.
3. Choose **No LLM** to use speech recognition and basic cleanup alone. If you want AI rewriting, select an LLM and explicitly install it in Performance.

No model weights are bundled with these installers. The builds and release checks do not download models. `SHA256SUMS.txt` contains checksums for the download files.

The release is published only after CI, all installer builds, Android signature verification, and the complete asset inventory pass. Automated tests do not replace physical microphone, hotkey, or mobile-device testing.

For rollback, back up local settings/history and reinstall [0.2.0](https://github.com/Lawliet2004/reflow/releases/tag/v0.2.0). Keep the Android signing identity for future updates; a debug APK signed with a different key may need to be uninstalled before installing this release.
