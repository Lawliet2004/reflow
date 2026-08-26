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

Requires JDK 17 and Android SDK 35. The Gradle wrapper jar is not committed; Android Studio will generate it on first sync.
