# Build Specification — Local Wispr Flow-Style Desktop Dictation Application

## 1. Project Overview

Build a production-quality, open-source desktop voice-to-text application inspired by the user experience and workflow of Wispr Flow, but designed around a fundamentally different principle:

**All speech recognition should happen locally on the user's computer by default. No audio should be sent to a cloud API.**

The application should feel extremely lightweight, instantaneous, polished, modern, and unobtrusive.

The application is a system-wide dictation tool rather than merely a text editor with speech recognition.

The user should be able to:

1. Press a configurable global keyboard shortcut.
2. Speak naturally.
3. See the application enter a listening/recording state immediately.
4. Have the microphone audio processed locally.
5. Receive a low-latency transcript from Qwen3-ASR-0.6B.
6. Optionally have the transcript locally normalized, punctuated, and formatted.
7. Have the final text inserted into the currently focused application.
8. Release the shortcut and immediately stop recording.
9. View the resulting transcription in application history.
10. Delete individual history items or clear all history.
11. Configure microphone, language, hotkeys, appearance, model behavior, history retention, and performance options.
12. Run the application almost entirely from the system tray without keeping a large UI window open.

The application must prioritize:

**Latency > responsiveness > reliability > accuracy > resource efficiency > visual complexity.**

However, transcription accuracy must remain high enough to make the application useful as a daily driver.

---

# 2. Core Technology Stack

## Desktop framework

Use:

* Tauri 2
* Rust backend
* TypeScript frontend
* Modern lightweight frontend framework such as React + Vite, or another Tauri-compatible TypeScript framework
* CSS with a lightweight design system
* SQLite for local history/settings metadata where appropriate

Tauri should be used for:

* Native application lifecycle
* Global shortcuts
* System tray
* Window management
* Native filesystem operations
* Native notifications
* Application settings
* Platform integration
* Native text insertion integration
* IPC between frontend and Rust
* Model process management

The frontend should never directly manage operating-system functionality that can be handled safely by Rust.

Tauri's architecture should remain the boundary between the UI and native functionality.

---

# 3. Primary ASR Model

Use:

**Qwen/Qwen3-ASR-0.6B**

Do not substitute Whisper as the primary model.

Do not use an external cloud transcription API in the default architecture.

The ASR system must support:

* Local inference
* Offline operation
* Streaming recognition
* English
* Hindi
* Bengali if supported by the selected runtime/model configuration; if Bengali support is unavailable in the selected official model build, clearly expose that limitation and support model/runtime upgrades without redesigning the application
* Automatic language identification where supported
* Long-form speech
* No Internet dependency after model installation

The official Qwen3-ASR family supports offline and streaming inference, and the 0.6B version is intended as an accuracy/efficiency trade-off.

Use the **0.6B** model as the default because the application is explicitly designed for:

* low memory
* low latency
* laptop-class hardware
* local inference
* rapid startup

---

# 4. ASR Runtime Architecture

Do NOT tightly couple the Tauri UI to the ASR implementation.

Create a dedicated local inference layer.

Preferred architecture:

Tauri Rust Core
→ Audio Engine
→ VAD
→ ASR Runtime Manager
→ Local Qwen3-ASR Runtime
→ Partial Transcript Stream
→ Transcript Processor
→ Text Injection Engine

The ASR runtime should operate as an isolated component.

Possible implementations should be evaluated in this order:

1. Native/local optimized runtime if sufficiently mature.
2. Official Qwen3-ASR runtime exposed through a local service/process.
3. Lightweight sidecar process controlled by Rust.
4. Python runtime only when necessary.

Do not put the entire application into Python.

The desktop application should remain primarily Rust + TypeScript.

The model runtime should be replaceable without rewriting the application.

Create an abstraction:

```text
ASREngine
 ├── initialize()
 ├── load_model()
 ├── unload_model()
 ├── start_stream()
 ├── push_audio()
 ├── get_partial_transcript()
 ├── get_final_transcript()
 ├── stop_stream()
 ├── cancel_stream()
 ├── get_language()
 └── get_metrics()
```

The frontend must not know whether ASR is implemented through Python, ONNX, CUDA, CPU inference, a native runtime, or another backend.

---

# 5. Streaming Architecture

The most important property of this application is perceived latency.

Do not wait until the user finishes speaking before beginning inference.

Pipeline:

```text
Microphone
    ↓
Audio Capture
    ↓
Resampling
    ↓
Voice Activity Detection
    ↓
Audio Chunk Buffer
    ↓
Streaming ASR
    ↓
Partial Transcript
    ↓
Transcript Stabilization
    ↓
Formatting / Cleanup
    ↓
Final Transcript
    ↓
Text Injection
```

The system should start processing speech while the user is still speaking.

Implement:

* continuous microphone capture while recording
* small audio chunks
* incremental processing
* transcript stabilization
* partial result replacement
* final result commitment
* silence detection
* automatic stopping after configurable silence
* manual stopping
* cancellation

Do not redraw the entire transcript for every token.

Only update the changed segment.

---

# 6. Latency Targets

Define measurable performance goals.

Target:

### Recording start

Global shortcut → microphone activated:

**< 100 ms**

### First transcript feedback

Speech begins → first visible partial transcript:

**Target < 500 ms**

### Streaming update

Additional speech → partial update:

**Target < 200–300 ms**

### Final transcript

Speech stops → final committed text:

**Target < 500 ms**

### Text insertion

Final text ready → target application receives text:

**Target < 100 ms**

These are engineering targets rather than guarantees.

Implement instrumentation so actual performance can be measured.

Display optional developer metrics:

```text
Input latency
VAD latency
ASR latency
Post-processing latency
Injection latency
Total latency
CPU usage
RAM usage
GPU usage
Model load time
```

---

# 7. Audio Engine

Implement native audio capture through the Rust backend.

Do not route audio through the TypeScript frontend.

The frontend should receive state/events, not raw microphone PCM data.

Required features:

* default microphone
* microphone selection
* sample-rate handling
* mono conversion
* resampling
* device hot-plug handling
* device disconnect recovery
* input level monitoring
* clipping detection
* silence detection
* microphone permission handling
* audio stream recovery

Preferred internal audio format:

```text
mono
16 kHz
16-bit PCM
```

Convert device-native formats internally.

Avoid unnecessary audio format conversions.

---

# 8. Voice Activity Detection

Implement a low-cost VAD before ASR.

Goals:

* avoid running ASR on silence
* reduce CPU/GPU usage
* detect beginning of speech quickly
* detect end of speech reliably
* avoid cutting off the beginning/end of words

Configurable settings:

```text
Speech threshold
Silence threshold
Start padding
End padding
Minimum speech duration
Maximum recording duration
Auto-stop silence duration
```

Recommended behavior:

* Begin recording immediately after hotkey press.
* VAD determines whether meaningful speech is present.
* Maintain a short pre-roll buffer so the first phonemes are not lost.
* Keep a short post-roll buffer when speech ends.
* Do not aggressively terminate during natural pauses.

---

# 9. Global Hotkey System

The hotkey system is a core feature.

Default behavior:

### Push-to-talk

Hold:

```text
Ctrl + Space
```

The application records while held.

Release:

```text
Ctrl + Space
```

Recording ends and final text is inserted.

Also support:

### Toggle mode

Press shortcut once:

```text
Listening
```

Press again:

```text
Stop
```

Allow users to configure:

* Ctrl + Space
* Alt + Space
* Ctrl + Shift + Space
* F-keys
* custom combinations
* mouse button combinations where supported

Detect conflicts.

Display:

```text
Shortcut already used by another application
```

Allow the user to replace the shortcut.

Implement all shortcut handling in Rust.

---

# 10. User Experience

The application should behave more like a system utility than a conventional desktop application.

When idle:

* no large window
* minimal system resource usage
* system tray icon
* background-ready
* model optionally kept loaded

When recording:

Display a small floating overlay near the bottom-center/top-center of the screen.

Example:

```text
┌────────────────────────────────────┐
│     ● Listening...                 │
│                                    │
│  "I need to finish this project..."│
└────────────────────────────────────┘
```

The overlay should be:

* borderless
* semi-transparent
* compact
* animated subtly
* keyboard-focus independent
* always-on-top during recording
* non-interactive by default

Do not make the application feel like a traditional media recorder.

---

# 11. Wispr Flow-Style Interaction

The product should recreate the interaction model, not copy branding or copyrighted UI.

Core experience:

```text
Press shortcut
↓
Instant listening
↓
Speak normally
↓
Live recognition
↓
Speak with natural pauses
↓
Release shortcut
↓
Clean formatted text appears in focused application
```

Users should not need to manually copy and paste in the common case.

The app should support:

* browser text boxes
* VS Code
* terminals
* IDEs
* Notion
* Word
* Slack
* Discord
* chat applications
* email applications
* arbitrary native text fields

---

# 12. Text Injection Engine

Implement a dedicated Rust text injection subsystem.

Primary methods:

1. Clipboard-based insertion
2. Native accessibility/text APIs where available
3. Keyboard simulation fallback

Preferred behavior:

```text
Save clipboard
↓
Set clipboard to generated text
↓
Paste into focused application
↓
Restore user's clipboard
```

Clipboard restoration must happen safely.

If restoration fails, do not destroy the user's previous clipboard contents intentionally.

Support:

* Unicode
* Bengali
* Hindi
* English
* emojis
* punctuation
* line breaks
* code snippets
* symbols

Do not inject text character-by-character unless necessary.

Prefer atomic text insertion.

---

# 13. Transcript Processing

Raw speech recognition should not necessarily be inserted directly.

Create a local transcript-processing stage.

Responsibilities:

* capitalization
* punctuation
* paragraph breaks
* removal of filler words where appropriate
* false-start cleanup
* repeated-word cleanup
* number formatting
* common abbreviation formatting
* date formatting
* time formatting
* currency formatting
* sentence boundaries
* preservation of intentional wording

Important:

Do not rewrite the user's meaning.

For example:

Raw:

```text
um I think we should probably push this tomorrow uh because the API isn't ready
```

Output:

```text
I think we should probably push this tomorrow because the API isn't ready.
```

But:

```text
I don't want this.
```

must never become:

```text
I want this.
```

The processing layer must prioritize semantic preservation.

---

# 14. Local Post-Processing Architecture

Create an optional second-stage local language model.

Do not make this mandatory for basic transcription.

Architecture:

```text
ASR transcript
      ↓
Transcript Processor
      ↓
Optional local formatting model
      ↓
Final text
```

The application should work without this component.

This keeps the base application lightweight.

Allow three modes:

### Raw

Insert the ASR result exactly as produced.

### Smart

Perform lightweight deterministic cleanup.

### Flow

Use a local language model for more advanced formatting and cleanup.

The user should be able to choose.

---

# 15. Context-Aware Formatting

Allow users to configure application-specific behavior.

For example:

### In a terminal

Preserve:

```text
npm install
git commit -m "message"
cargo build
```

Do not convert technical syntax into prose.

### In an IDE

Preserve:

* function names
* variable names
* file paths
* commands
* programming terminology
* URLs

### In email

Favor:

* complete sentences
* punctuation
* paragraphs

### In chat

Favor:

* natural conversational formatting

### In documentation

Favor:

* paragraphs
* punctuation
* headings where explicitly dictated

Implement optional application profiles.

---

# 16. History System

Every completed transcription should optionally be stored locally.

Use SQLite.

Store:

```text
id
created_at
duration_ms
language
raw_transcript
final_transcript
application_name
application_process
word_count
character_count
model_version
processing_mode
```

Do not store raw audio by default.

Audio storage must be explicitly opt-in.

Default:

```text
Audio retention = OFF
Transcript history = ON
```

Allow:

```text
History retention:
- Disabled
- 1 day
- 7 days
- 30 days
- 90 days
- Forever
```

---

# 17. History UI

Create a dedicated history page.

Example:

```text
History

Today
────────────────────────
"I need to finish the API..."
10:42 AM

"Can you check the deployment..."
9:14 AM

Yesterday
────────────────────────
"We should migrate this..."
6:32 PM
```

Each entry should support:

* copy
* insert again
* edit
* delete
* view details
* show timestamp
* show duration
* show language

Search:

```text
Search transcripts...
```

Search should work locally.

Never send history to a server.

---

# 18. History Deletion

Support:

### Delete one

Delete a single transcription.

### Delete selected

Delete multiple entries.

### Delete today's history

Remove all today's records.

### Clear all history

Delete everything.

Before destructive deletion:

```text
Delete all history?

This cannot be undone.
```

Provide:

```text
Cancel
Delete
```

---

# 19. Privacy Architecture

Privacy should be a first-class design principle.

Default rules:

* No cloud speech recognition.
* No uploaded audio.
* No telemetry containing transcript contents.
* No transcript analytics.
* No remote transcription API.
* No automatic audio storage.
* No third-party server required for basic operation.

Network access should not be required for:

* recording
* transcription
* history
* text insertion
* configuration

Internet access may optionally be used for:

* application update checks
* model downloads
* release notifications

Provide:

```text
Offline Mode
```

When enabled, the app must make no network calls.

---

# 20. Model Installation

The application should not require users to manually configure Python.

Create a model manager.

First launch:

```text
Welcome

Choose speech recognition model

Qwen3-ASR 0.6B
~[calculated package size]

[Download Model]
```

Show:

```text
Download progress
Verification
Installation
Initialization
```

Verify model checksums.

Store models in an application-managed directory.

Allow:

* install
* uninstall
* update
* verify
* repair
* switch model
* display model version

Never redownload an existing valid model.

---

# 21. Model Loading

Implement intelligent model lifecycle management.

Modes:

### Always loaded

Lowest latency.

Higher memory usage.

### Load on first use

Lower idle memory.

Slight first-use latency.

### Unload after inactivity

Unload after configurable:

```text
1 minute
5 minutes
15 minutes
30 minutes
```

Recommended default:

**Keep the model loaded while the application is active.**

Provide the setting:

```text
Optimize for:
○ Lowest latency
○ Lowest memory
```

---

# 22. Hardware Acceleration

Detect available hardware.

Support, where the selected runtime permits:

* CPU
* NVIDIA GPU
* other supported accelerators

At startup:

```text
Hardware
CPU: Ryzen 5 7535HS
GPU: RTX 2050
VRAM: 4 GB
```

Select the safest supported execution backend automatically.

Allow manual override:

```text
Automatic
CPU
GPU
```

Never assume GPU availability.

The application must remain functional on CPU-only systems.

---

# 23. Resource Targets

The application itself should remain lightweight.

Do not load large unnecessary frontend dependencies.

Avoid:

* Electron
* Chromium runtime bundling
* large embedded databases
* background services that aren't needed
* multiple JavaScript worker processes
* duplicate audio buffers

Target idle Tauri application memory:

**as low as practically achievable, ideally well below 150 MB excluding the ASR model/runtime.**

The model memory must be reported separately.

Display:

```text
Application RAM
Model RAM
Total RAM
```

Storage should also be reported separately:

```text
Application
Models
History
Logs
Cache
```

---

# 24. Startup Optimization

Cold startup target:

**< 1 second before UI/system integration is ready**, excluding model loading.

Do not block application startup on model initialization.

Startup sequence:

```text
Launch application
↓
Initialize Rust
↓
Initialize tray
↓
Register global shortcut
↓
Initialize audio subsystem
↓
Show app ready
↓
Load model asynchronously
↓
Mark ASR ready
```

The user should be able to open settings immediately even if the model is still loading.

---

# 25. State Machine

Implement an explicit state machine.

States:

```text
UNINITIALIZED
INITIALIZING
IDLE
LOADING_MODEL
READY
RECORDING
PROCESSING
INJECTING
ERROR
UPDATING
```

Example:

```text
IDLE
 ↓
shortcut pressed
 ↓
RECORDING
 ↓
speech detected
 ↓
STREAMING
 ↓
shortcut released
 ↓
FINALIZING
 ↓
INJECTING
 ↓
HISTORY_SAVE
 ↓
IDLE
```

Never manage this solely through scattered frontend booleans.

Keep authoritative state in Rust.

---

# 26. Frontend State

Frontend should receive events such as:

```text
app:ready
model:loading
model:ready
recording:started
recording:audio-level
transcript:partial
transcript:updated
transcript:final
injection:started
injection:complete
recording:error
model:error
```

Use strongly typed TypeScript interfaces.

Never use arbitrary string payloads everywhere.

Define a typed event schema.

---

# 27. Rust ↔ TypeScript API

Create a clean Tauri command layer.

Examples:

```text
get_app_state()
get_settings()
update_settings()
get_audio_devices()
set_audio_device()
start_recording()
stop_recording()
cancel_recording()
get_history()
search_history()
delete_history_item()
clear_history()
copy_history_item()
inject_text()
get_model_status()
install_model()
remove_model()
reload_model()
get_performance_metrics()
```

Commands should be asynchronous where appropriate.

Never block the Tauri UI thread.

---

# 28. Error Handling

The app should never silently fail.

Possible errors:

```text
Microphone unavailable
Microphone permission denied
Shortcut registration failed
Model missing
Model corrupted
Model failed to load
Insufficient RAM
GPU unavailable
ASR runtime crashed
Text insertion failed
Clipboard unavailable
Target application inaccessible
Audio device disconnected
```

Provide useful recovery actions.

Example:

```text
Qwen3-ASR could not be loaded.

Possible causes:
• Model files are incomplete.
• Not enough memory.
• Unsupported execution backend.

[Repair Model]
[Change Backend]
[Open Logs]
```

---

# 29. Crash Recovery

If the ASR runtime crashes:

1. Record crash.
2. Keep the main application alive.
3. Restart the runtime.
4. Reinitialize model.
5. Report status to user.

Never terminate the entire desktop app because the model process crashed.

---

# 30. Logging

Use structured local logs.

Example:

```text
2026-08-21 16:30:41 INFO application_started
2026-08-21 16:30:42 INFO model_loaded model=qwen3-asr-0.6b
2026-08-21 16:31:05 INFO recording_started
2026-08-21 16:31:08 INFO recording_finished duration_ms=3201
2026-08-21 16:31:09 INFO transcript_completed latency_ms=412
```

Never put full transcripts into ordinary diagnostic logs unless explicitly enabled.

Do not log audio.

Provide:

```text
Settings
→ Advanced
→ Open Logs Folder
```

---

# 31. Settings

Create a full settings system.

## General

* Launch at startup
* Start minimized
* Start in tray
* Keep model loaded
* Automatically check updates
* Theme

## Dictation

* Hotkey
* Push-to-talk
* Toggle mode
* Auto-stop
* Maximum duration
* Language
* Automatic language detection

## Microphone

* Input device
* Input gain
* Noise suppression
* VAD sensitivity

## AI

* Model
* Compute backend
* CPU/GPU
* Processing mode
* Formatting enabled
* Context-aware formatting

## History

* Enable history
* Retention duration
* Store audio
* Maximum history size

## Appearance

* Overlay position
* Overlay size
* Animation
* Compact mode
* Theme

## Advanced

* Debug logs
* Performance metrics
* Model cache
* Runtime configuration

---

# 32. Language Handling

Since the target user speaks Bengali, Hindi, and English, design language selection carefully.

Modes:

### Auto

Detect language automatically.

### English

Force English.

### Hindi

Force Hindi.

### Bengali

Force Bengali.

### Multi-language

Allow the model to identify language dynamically.

For the common case, recommend:

```text
Auto
```

But allow English to be explicitly selected for users who mostly speak English and want to minimize unnecessary language-identification overhead.

Do not force a language switch after every sentence unless the ASR runtime supports reliable multilingual recognition.

---

# 33. Mixed-Language Speech

Support code-switching as a first-class use case.

Example:

```text
Actually ami kalke office jabo because I need to finish this project.
```

Do not aggressively translate speech.

Transcribe the language actually spoken.

The application should preserve English technical vocabulary inside Hindi/Bengali speech.

Example:

```text
আজকে আমাকে APIটা ঠিক করতে হবে
```

should remain a mixed-language sentence rather than being translated into English.

---

# 34. Dictionary / Custom Vocabulary

Implement a custom vocabulary system.

User can add:

```text
OpenAI
Qwen
Tauri
Rust
TypeScript
PostgreSQL
Supabase
LangGraph
GitHub
IEM
```

The system should use these terms to improve post-processing and correction where possible.

Allow:

```text
Term
Preferred spelling
Category
```

Example:

```text
Qwen → Qwen
Tauri → Tauri
Type Script → TypeScript
Git Hub → GitHub
```

---

# 35. Custom Replacements

Implement deterministic replacements.

Example:

```text
"Git hub" → "GitHub"
"VS code" → "VS Code"
"Type script" → "TypeScript"
```

Allow user-defined replacements.

Provide:

```text
Before
After
```

---

# 36. Smart Formatting

Recognize speech patterns such as:

```text
new line
new paragraph
comma
period
question mark
colon
semicolon
open quote
close quote
```

Do not require users to say punctuation under normal circumstances.

Infer punctuation automatically.

---

# 37. Special Modes

Implement optional dictation modes.

### Normal

General conversational text.

### Coding

Preserve:

* code
* APIs
* identifiers
* commands
* technical terminology

### Email

Structured professional prose.

### Chat

Natural conversational formatting.

### Notes

Paragraph-oriented transcription.

Do not artificially rewrite text unless the mode requires formatting.

---

# 38. Overlay Design

Visual style:

**Minimal, premium, modern, dark/light adaptive, soft rounded geometry, subtle motion.**

Do not clone Wispr Flow exactly.

Create original visual branding.

Recording overlay:

```text
╭──────────────────────────────────╮
│  ●  Listening                    │
│                                  │
│  I need to finish the project... │
╰──────────────────────────────────╯
```

During processing:

```text
╭──────────────────────────────────╮
│  ◌  Processing                   │
╰──────────────────────────────────╯
```

On completion:

```text
╭──────────────────────────────────╮
│  ✓  Inserted                     │
╰──────────────────────────────────╯
```

Animations should be subtle and GPU-friendly.

---

# 39. System Tray

Tray menu:

```text
Application Name

● Ready

Start Dictation
Pause
History
Settings

Microphone
    Default Microphone

Language
    Auto
    English
    Hindi
    Bengali

Model
    Qwen3-ASR 0.6B

Quit
```

Do not expose unnecessary technical information in the normal tray menu.

---

# 40. Accessibility

Support:

* keyboard-only usage
* screen readers for settings
* high contrast
* scalable text
* reduced motion
* accessible focus order

The dictation workflow itself should not require mouse interaction.

---

# 41. Security

Apply least privilege.

Do not request administrator privileges unless absolutely necessary.

Protect:

* local database
* configuration
* model files
* logs

Avoid storing secrets.

Use OS-native secure storage if future cloud integrations introduce API keys.

---

# 42. Data Directory

Use platform-appropriate application directories.

Example:

```text
app_data/
├── database/
│   └── history.db
├── models/
│   └── qwen3-asr-0.6b/
├── logs/
├── cache/
└── config/
```

Do not store application data beside the executable.

---

# 43. Project Structure

Use a clean monorepo structure.

Example:

```text
project/
├── src/
│   ├── frontend/
│   │   ├── components/
│   │   ├── pages/
│   │   ├── hooks/
│   │   ├── stores/
│   │   ├── services/
│   │   ├── types/
│   │   └── styles/
│   │
│   └── ...
│
├── src-tauri/
│   ├── src/
│   │   ├── audio/
│   │   ├── asr/
│   │   ├── hotkey/
│   │   ├── injection/
│   │   ├── history/
│   │   ├── model/
│   │   ├── settings/
│   │   ├── tray/
│   │   ├── platform/
│   │   ├── logging/
│   │   └── main.rs
│   │
│   └── Cargo.toml
│
├── model-runtime/
├── scripts/
├── tests/
├── docs/
└── README.md
```

---

# 44. Separation of Responsibilities

Frontend:

* UI
* settings
* history display
* animations
* state visualization

Rust:

* native integrations
* audio capture
* global shortcuts
* model lifecycle
* ASR communication
* text injection
* database access
* filesystem
* tray
* performance metrics

ASR runtime:

* acoustic processing
* streaming transcription
* model execution

Never make React/TypeScript responsible for real-time audio processing.

---

# 45. Performance Rules

The implementation must follow these rules:

1. No unnecessary allocations in the audio loop.
2. Reuse audio buffers.
3. Reuse transcription buffers.
4. Avoid copying PCM data unnecessarily.
5. Avoid sending raw audio through the WebView.
6. Batch UI events when appropriate.
7. Do not emit dozens of frontend events every millisecond.
8. Update the transcript at a human-perceivable frequency.
9. Avoid constant database writes while recording.
10. Write history only after finalization.
11. Keep model initialization off the UI thread.
12. Keep the ASR runtime isolated.
13. Avoid unnecessary background threads.
14. Shut down unused resources.
15. Avoid loading history entries unnecessarily.

---

# 46. Transcript Stabilization

Streaming ASR may change recent words as additional audio arrives.

Never visually flicker the entire sentence.

Maintain:

```text
Committed prefix
+
Mutable suffix
```

Example:

```text
Committed:
I need to finish the

Mutable:
API project tomorrow
```

If the ASR revises the last words, only update the mutable suffix.

When speech ends:

```text
Commit everything.
```

---

# 47. Latency Instrumentation

Every recording should internally measure:

```text
hotkey_to_recording
recording_to_first_audio
audio_to_first_partial
audio_to_first_stable_word
speech_end_to_final
final_to_injection
total_time
```

Developer diagnostics should show:

```text
Session

Audio duration: 7.8 s
First partial: 340 ms
Final ASR: 281 ms
Formatting: 47 ms
Injection: 31 ms
Total: 398 ms
```

Use these metrics to optimize the application.

---

# 48. Testing

Create automated tests for:

### Audio

* device selection
* device disconnect
* silence
* clipping
* resampling

### Hotkeys

* registration
* conflict
* push-to-talk
* toggle mode

### ASR

* model loading
* model unloading
* streaming
* cancellation
* runtime crash recovery

### Language

* English
* Hindi
* Bengali where supported by the selected ASR build
* mixed-language input

### Injection

* Unicode
* clipboard restoration
* terminal
* browser
* IDE
* native applications

### History

* creation
* search
* deletion
* retention
* migration

### Performance

Benchmark:

* idle RAM
* loaded-model RAM
* CPU
* GPU
* startup
* model load
* first partial latency
* final latency

---

# 49. Test Corpus

Create a local benchmark suite containing:

### English

* conversational English
* technical English
* fast English
* accented English
* noisy environments
* low-volume speech
* long sentences

### Hindi

* formal Hindi
* conversational Hindi
* Hindi with English technical vocabulary

### Bengali

* conversational Bengali
* Bengali with English technical vocabulary

### Mixed

```text
আজকে I need to deploy the application
```

```text
कल हमें API को fix करना है
```

Measure:

```text
WER
CER
Latency
Word stability
Formatting accuracy
```

---

# 50. Application Profiles

Allow profile-based behavior.

Example:

```text
Chrome
→ Normal

VS Code
→ Coding

Windows Terminal
→ Coding

Notion
→ Notes

Slack
→ Chat
```

Detect active application.

Never transmit application content to a server.

Only use locally available process/window metadata.

---

# 51. Privacy-Safe Context

Do not automatically read everything from the active application.

Instead, support optional future context providers.

For example:

```text
Context permission:
Disabled
Application name only
Selected text
Explicitly provided context
```

Default:

**Disabled.**

This keeps the local-first privacy model intact.

---

# 52. Model Update System

Model manager should support:

```text
Current Model:
Qwen3-ASR-0.6B

Version:
...

Size:
...

Runtime:
...

Backend:
...

Status:
Ready
```

Allow model upgrades without requiring an application upgrade when possible.

Keep model versions isolated:

```text
models/
├── qwen3-asr-0.6b-v1/
├── qwen3-asr-0.6b-v2/
└── active -> v2
```

Allow rollback.

---

# 53. Offline-First Installation

The application must remain usable after installation without Internet.

Provide two installation modes:

### Online

Download model during setup.

### Offline

Allow the user to provide a local model package.

Verify checksums before activation.

---

# 54. Packaging

Build installers for:

* Windows
* Linux
* macOS

Prioritize Windows first.

Windows should have:

* native installer
* Start Menu shortcut
* Start-on-login option
* system tray
* global shortcuts
* native text insertion

---

# 55. Windows Priority

Since the primary target is desktop productivity, Windows integration should be excellent.

Support:

* Windows microphone permissions
* global keyboard shortcuts
* clipboard
* SendInput where needed
* focused-window detection
* system tray
* auto-start
* Windows notifications

Avoid requiring administrator mode.

---

# 56. Model Footprint Strategy

The base application should remain small.

Treat:

```text
Application
```

and:

```text
AI model
```

as separate downloadable components.

This allows the binary to stay lightweight.

Display:

```text
App: 8 MB
Model: 1.9 GB
Total installed: 1.91 GB
```

The actual numbers must be obtained dynamically rather than hardcoded.

The official unquantized Qwen3-ASR-0.6B repository currently shows a roughly 1.88 GB safetensors model file, so storage optimization should be investigated through supported quantized/runtime-specific distributions rather than pretending the base checkpoint is only a few hundred megabytes.

Where possible, support a smaller optimized local model package.

Do not sacrifice reliability solely to minimize the model by a few hundred megabytes.

---

# 57. Runtime Selection

Create an internal interface:

```text
RuntimeBackend

CPU
GPU
OptimizedCPU
OptimizedGPU
```

The application should detect the best supported backend.

The user should not need to understand:

* CUDA
* PyTorch
* vLLM
* tensor formats
* model weights
* inference libraries

Those should remain implementation details.

---

# 58. Developer Mode

Add:

```text
Settings
→ Advanced
→ Developer Mode
```

Developer mode exposes:

* live latency
* real-time CPU
* RAM
* GPU
* audio levels
* model state
* backend
* runtime logs
* ASR partial results
* event stream

Provide:

```text
Copy diagnostics
```

Never include raw audio.

Avoid transcript contents in diagnostics unless explicitly selected.

---

# 59. CLI

Create an optional CLI:

```text
app status
app model list
app model install qwen3-asr-0.6b
app model remove qwen3-asr-0.6b
app transcribe file.wav
app history list
app history clear
app benchmark
```

This is useful for debugging and automation.

The GUI and CLI should use the same Rust core.

---

# 60. API Stability

Do not allow frontend implementation details to leak into Rust.

Create stable internal interfaces:

```text
AudioProvider
ASREngine
ModelManager
TranscriptProcessor
TextInjector
HistoryStore
HotkeyManager
SettingsStore
PlatformAdapter
```

Each must be independently testable.

---

# 61. Platform Abstraction

Create:

```text
PlatformAdapter
├── WindowsAdapter
├── MacOSAdapter
└── LinuxAdapter
```

The rest of the application should not contain platform-specific conditional logic everywhere.

Put it behind platform abstractions.

---

# 62. Future Features

Design the architecture so these can be added later without replacing the core:

* multiple ASR models
* larger accuracy model
* custom models
* cloud fallback
* user-defined API providers
* AI rewrite commands
* summarize dictation
* translate dictation
* voice commands
* punctuation commands
* command mode
* context-aware rewriting
* per-application profiles
* model benchmarking
* model marketplace
* plugins

Do not implement these in v1 unless required.

---

# 63. Explicit Non-Goals for v1

Do NOT initially implement:

* cloud transcription
* accounts
* social features
* telemetry
* online transcript storage
* collaboration
* complicated AI agent features
* unnecessary animations
* giant settings pages
* automatic uploading
* mandatory account creation

The first version must excel at:

**Press → speak → text appears.**

---

# 64. V1 Feature Set

The first functional release must include:

### Core

* Tauri 2
* Rust
* TypeScript
* Qwen3-ASR-0.6B
* local inference
* microphone capture
* VAD
* streaming transcription
* global hotkey
* push-to-talk
* toggle mode
* low-latency partial transcript
* final transcript
* text injection
* system tray

### Storage

* SQLite
* local history
* search
* delete
* clear-all
* retention settings

### UI

* floating recording overlay
* history
* settings
* model status
* microphone settings
* hotkey configuration
* language configuration

### Reliability

* runtime recovery
* device recovery
* model recovery
* logging
* error reporting

---

# 65. Development Phases

Implement in this order.

## Phase 1 — Native skeleton

Build:

```text
Tauri
+
Rust
+
TypeScript
+
Tray
+
Settings
+
Global shortcut
```

Do not integrate AI yet.

---

## Phase 2 — Audio

Implement:

```text
Microphone
→ Audio stream
→ Resampling
→ VAD
```

Build a microphone test screen.

---

## Phase 3 — Qwen3-ASR

Integrate:

```text
Qwen3-ASR-0.6B
```

Implement:

```text
model loading
streaming
partial transcript
final transcript
runtime restart
```

---

## Phase 4 — Text insertion

Implement:

```text
transcript
→ clipboard/native input
→ focused application
```

Test against:

* Notepad
* Chrome
* VS Code
* Terminal

---

## Phase 5 — Wispr-style workflow

Combine:

```text
hotkey
→ recording
→ VAD
→ ASR
→ finalization
→ text injection
```

Make the workflow extremely fast.

---

## Phase 6 — History

Add:

```text
SQLite
search
delete
retention
history UI
```

---

## Phase 7 — Formatting

Add:

```text
punctuation
capitalization
cleanup
custom replacements
application profiles
```

---

## Phase 8 — Optimization

Measure:

```text
RAM
CPU
startup
model load
first partial
final latency
injection latency
```

Optimize based on measured data.

Do not optimize based on assumptions.

---

## Phase 9 — Packaging

Create:

```text
Windows installer
Linux package
macOS package
```

Ensure the application survives:

* restart
* sleep/wake
* microphone changes
* model crashes
* GPU changes

---

# 66. Acceptance Criteria

The application is considered successful when:

### User experience

A user can:

1. Launch the app.
2. Configure microphone.
3. Configure shortcut.
4. Start dictation from anywhere.
5. Speak naturally.
6. See near-real-time transcription.
7. Release the hotkey.
8. Have formatted text inserted into the focused application.
9. Open history.
10. Search history.
11. Delete entries.
12. Run without Internet.

### Performance

The application should strive for:

```text
App startup: < 1 s
Shortcut response: < 100 ms
First transcript: < 500 ms
Finalization: < 500 ms
Text insertion: < 100 ms
```

Again, these are engineering targets.

### Privacy

No audio leaves the computer during normal operation.

### Reliability

A failed ASR runtime must not kill the main application.

### Resource efficiency

The frontend must remain lightweight and the model runtime should consume only the resources required by the selected inference backend.

---

# 67. Definition of Done

Do not mark the feature complete because the application "works."

A feature is complete only when:

* implemented
* tested
* measured
* recoverable from errors
* integrated into the settings UI
* documented
* cross-platform behavior considered
* no unnecessary resource consumption exists
* privacy implications are documented

---

# 68. Final Product Vision

The finished application should feel like a native operating-system utility.

The user should not think:

"I am using an AI transcription application."

They should think:

"I pressed a shortcut and my computer understood what I said."

The ideal flow is:

```text
Anything I am doing
       ↓
    Shortcut
       ↓
  Speak naturally
       ↓
 Local Qwen3-ASR
       ↓
 Low-latency transcript
       ↓
 Smart local formatting
       ↓
 Text insertion
       ↓
   Back to work
```

The core engineering philosophy is:

**Local first.**
**Fast first.**
**Private by default.**
**Low memory.**
**No cloud dependency.**
**No unnecessary UI.**
**No unnecessary processing.**
**The model should disappear into the experience.**

Build the MVP around Qwen3-ASR-0.6B first. Do not build a complex AI stack before proving that the basic loop — hotkey → microphone → streaming ASR → text insertion — feels instantaneous and reliable.

Use the Qwen3-ASR model/runtime as a replaceable backend because the ecosystem is actively evolving. The official project already provides streaming and offline inference paths, and native Transformers support was added in 2026, so the implementation should avoid hard-coding the architecture around one inference mechanism.

For the first working milestone, the absolute priority is:

**Global hotkey → local Qwen3-ASR-0.6B → low-latency text → focused application.**
