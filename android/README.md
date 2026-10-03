# Reflow for Android

Kotlin + Jetpack Compose companion. The phone records 16 kHz PCM and streams it to the **desktop** LAN API. Qwen3-ASR does not run on the phone.

## Pair

1. Run Reflow on Windows or Linux.
2. Settings → Android / LAN API → enable, note IP + 6-digit code.
3. Open this app, enter IP/port/code (or open a `reflow://pair?...` QR).

## Build

Open the `android/` folder in Android Studio (Ladybug/Koala or newer) or:

```bash
cd android
gradlew.bat assembleDebug
```

Requires JDK 17 and Android SDK 35. The Gradle wrapper jar is included.

Run local lifecycle/network regressions with `gradlew.bat testDebugUnitTest`. These mock the microphone hardware boundary; they do not replace device tests for permission prompts, rotation, microphone unplugging, foreground-service behavior, or network interruption.

Speech-language options are packaged from the same `model-runtime/languages.json` catalog used by the desktop. Existing unsupported language preferences return to automatic detection.

Pairing accepts localhost and literal private IPv4 addresses. All requests and audio use HTTPS/WSS with the desktop's exact certificate SHA-256 pin and certificate validity checks. Copy a fresh `reflow://pair?v=2&...&cert_sha256=...` link from desktop Settings → Phone, or copy its displayed certificate fingerprint into manual setup. Legacy/pinless connections require re-pairing; there is no cleartext fallback. If the desktop certificate changes or expires, verify the desktop and pair again.

The pin is a SHA-256 digest of the DER leaf certificate, not an SPKI/public-key digest. It is the desktop identity, so moving that same identity to another allowed private IP does not require trusting a new certificate. Redirects are disabled to keep pairing credentials and tokens on the chosen endpoint.
