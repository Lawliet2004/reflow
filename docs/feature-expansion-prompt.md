# Implementation Prompt — Reflow Feature Expansion

You are implementing a large feature expansion for **Reflow**, a local-first,
offline desktop dictation app (Tauri 2 + Rust backend + React/TypeScript
frontend). Everything below must run **entirely on the user's machine**: no
accounts, no telemetry, no cloud calls beyond the existing pinned model/runtime
downloads. Latency priorities from `prompt.md` still hold:
**Latency > responsiveness > reliability > accuracy > resource efficiency.**

This is a multi-phase job. Implement phases **in order**, keep the app
shippable after every phase, and run the full check suite before marking a
phase done.

## Ground rules

1. **Offline guarantee.** The only outbound network calls that may exist after
   this work are the already-existing pinned downloads (HF model weights,
   llama.cpp runtime). Everything else — dictation, cleanup, commands,
   assistant, stats, notes — is local. Add no analytics, no update beacons, no
   crash reporting.
2. **KV-cache invariance.** The Stage-2 rewrite prompt prefix
   (`rewrite/prompt.rs` `BASE_SYSTEM` + `FEW_SHOT` via `cacheable_prefix()`)
   must stay byte-identical so llama.cpp reuses its KV cache. Any per-request
   variation (style, mode instructions, command prompts, context) goes in the
   **final user turn only**. See `rewrite/prompt.rs:94-119` for the rationale.
   New prompt families (command mode, assistant, summarize) get their own
   `build_*_messages()` functions and may define their own few-shot prefixes —
   they will miss the warm cache, which is acceptable, but keep their prefixes
   stable across calls too.
3. **Safety gates.** `accept_rewrite` (`rewrite/safety.rs:119`) validates
   _cleanup_ candidates (length bound, identifiers, numbers, script
   preservation, no meta-text, Levenshtein cap). It would reject legitimate
   _transformative_ output (translate, summarize, bullets). For command mode /
   custom modes create a lighter `accept_task_output`: non-empty, not
   meta/echo (`looks_like_meta`), reasonable length bound, not a refusal. Keep
   the strict gate for the dictation cleanup path unchanged.
4. **No new dependencies without justification.** Already available: `enigo`
   (cross-platform key synthesis incl. `Key::Return`, `Key::MediaPlayPause`,
   `text()` typing), `windows` 0.58 (extend its feature list for audio-session
   APIs), `hound` (WAV only), `axum`, `rusqlite`, `cpal` 0.15 (WASAPI loopback
   supported via output-device-as-input). Approved new deps when needed:
   `symphonia` (mp3/m4a/aac/flac/ogg decode for file transcription),
   `aes-gcm` + `keyring` (history encryption), `nnnoiseless` (optional noise
   suppression). Any other dep needs a comment in this file explaining why
   nothing installed covers it.
5. **Settings schema.** `AppSettings` is at `settings/config.rs` schema
   version 4 (`CURRENT_SETTINGS_VERSION`, `migrate_document`, defaults via
   `#[serde(default = …)]`). Bump to **5** with a `migrate_v4_to_v5` (mostly
   defaults-backfill). New fields need serde defaults so old documents
   deserialize. Mirror every new field in `src/types/index.ts`,
   `DEFAULT_SETTINGS` and `normalizeSettings` (`src/services/tauriApi.ts`).
   Settings writes go through `update_settings` IPC →
   `SettingsStore::merge_update` (top-level merge; nested sections like
   `asr`/`refinement` merge field-by-field — put new nested structs in
   `is_nested_section`). Side effects belong in `commands/mod.rs` under the
   `settings_operation` lock.
6. **History schema.** `history/db.rs` is at `user_version` 4. Bump to **5**:
   add columns via `if version < 5 { ALTER TABLE … }` inside the existing
   single-transaction migration, extend `HistoryEntry`, and update all SELECT
   column lists + `map_history_row` + `insert_entry` (see `db.rs:133, 477,
528, 559, 667`). The `history_with_audio` view uses `history.*` so new plain
   columns surface automatically; if you change its computed columns,
   DROP/CREATE it inside the migration.
7. **IPC plumbing.** Every new `#[tauri::command]` must be registered in the
   `invoke_handler!` list in `lib.rs:420-495` and wrapped in
   `src/services/tauriApi.ts` via `safeInvoke` with a sensible browser
   fallback (jsdom tests run with `isTauri()===false`). New frontend events go
   through `bind_dory_ui` (`lib.rs:500-586`) → `DoryEvent` (`dory/mod.rs`) and
   are consumed via `createEventScope`/`safeListen` (`src/services/eventScope.ts`).
8. **New settings pages.** Create `src/components/settings/<Name>Page.tsx`
   taking `{settings, onUpdateSettings}` built from `Section`/`Row`/`Toggle`
   (`settings/ui.tsx` re-exports), register `{id,label,description,keywords}`
   in `PAGES` + `PAGE_ICONS` in `settings/ui.tsx`, and add the render branch
   in `SettingsView.tsx:153-201`.
9. **New top-level views.** Extend `NavTab` in `Navigation.tsx:6`, add an
   `ITEMS` entry, add the render branch + scroll class in `App.tsx:369-453`.
10. **Tests are part of done.** Rust: `#[cfg(test)]` modules colocated +
    `src-tauri/tests/*.rs` integration files using `AppContext::bootstrap_test`
    and `AsrHandle::new_mock`/`swap_engine`. Frontend: colocated
    `*.test.tsx` with the `api.getSettings()` fixture and `vi.spyOn(api, …)`
    pattern. Python untouched unless the sidecar protocol changes (then update
    `model-runtime/test_runtime.py` and keep `--selftest` green).
11. **Definition of done per phase:** `cargo test --manifest-path
src-tauri/Cargo.toml`, `cargo clippy --manifest-path
src-tauri/Cargo.toml --all-targets -- -D warnings`, `npm run check`
    (lint + prettier + tsc + vitest) all green; the feature works end-to-end
    via the UI; README feature bullets updated.

## Known codebase anchors

- Stop path: `session.rs` `finish_stop` (632) → `postprocess_transcript`
  (126) → `format_transcript_ex` (formatting/mod.rs:143) → optional
  `polish_or_fallback` (rewrite/client.rs:573) → `TextInjector::inject`
  (injection/injector.rs:118) → history insert (session.rs:964-1006).
- Session entry points: `start_microphone_at` (session.rs:311),
  `spawn_start_at`/`spawn_stop_at`/`spawn_toggle_at` (commands/mod.rs:35-81).
  `CaptureKind` (dory/mod.rs:25): `None|Microphone|External`.
- Hotkeys: `register_dictation_hotkey` (commands/mod.rs:84) uses
  `tauri-plugin-global-shortcut` for normal combos and the Windows-only LL
  keyboard hook `hotkey/hook.rs` for modifier-only combos (single combo only,
  exact-match modifiers, ≥2 modifier families; `HookStateMachine` holds
  tap/double-tap/hands-free logic). The hook **observes** non-modifier keys in
  its `down` set even though they can't be part of a combo.
- Injection: clipboard snapshot/restore via `ClipboardAccess`
  (injection/clipboard.rs) + `RestoreChain`; Windows paste = raw `SendInput`
  Ctrl+V (platform/windows.rs:63-131); Linux = enigo Ctrl[+Shift]+V, then
  `wtype`, then `ydotool`; macOS inject is a stub (`Err`). `paste_chord_label`
  (platform/adapter.rs:216). No selected-text capture exists yet.
- Refinement runtime: `FlowRuntime::ensure(model, backend, layers, vram,
ctx)` → llama.cpp `llama-server` on a free loopback port,
  OpenAI-compatible `/v1/chat/completions`; `FlowClient::rewrite` is locked to
  `build_messages`; `context_size`/`completion_budget` bounds and the
  "input too long" sentence-split live in `client.rs:159-276,513-571`.
- Per-app profiles: `AppSettings::for_application` (config.rs:821) merges
  `application_profiles` (style + ≤60 dictionary terms) by process basename.
- API: axum HTTPS+WSS server (`api/server.rs`), routes `/v1/health|status|
pair|history|history/search|inject|transcribe|devices/:id|stream`; bearer
  tokens (sha256 stored), `DevicePermissions{stream,history,injection}`;
  `api_bind = "localhost"|"lan"`, port 7840; `blocking_job` semaphore (4).
  `session::transcribe_saved_audio` (session.rs:25) shows the
  feed-samples→stop_stream→transcript pattern to reuse.
- Audio: canonical 16 kHz mono f32. `wav_or_pcm_to_f32`
  (api/server.rs:702) decodes WAV (i16/f32, 1-8ch, 8-192k) → resample via
  `AudioResampler`; raw s16le fallback. `recording_pcm` ring buffer sized by
  `audio_retention`/`max_duration_sec` (session.rs:1096). ASR engines return
  plain text, no timestamps; sidecar internally cuts ≤30 s silence-aligned
  segments; native ≤8 s.
- Dead/unused hooks to leverage: `offline_mode` setting exists but does
  nothing (default `true` — wire it for real); `cancel_recording` command +
  `api.cancelRecording` binding exist but nothing calls them; `dictation_mode`
  field + `DictationMode` union exist (`coding` already gates Stage-2 off at
  session.rs:171/739); `undo_last_ai_edit`/`undo_history_ai_edit` commands
  exist with no UI; `app:auto-stop` and `hotkey:error` events are emitted but
  unconsumed; orphaned components `HardwareInspector`,
  `DiagnosticsWaterfall`, `CustomControls`, `TierCard`, `hud/ModeBadge`.
- Overlay: fixed 208×48 unfocusable always-on-top capsule
  (`overlay.rs:30`), shows phase label only; resize via `resize_overlay`.
- Tray: `MenuItem`s + `on_menu_event` in `lib.rs:307-405`; tray "history"/
  "settings" items only show the window — deep-linking a tab needs a new
  emitted event + `activeTab` wiring in App.tsx.
- macOS: injection unimplemented; treat all injection-dependent features as
  best-effort there (leave transcript on clipboard + message), matching
  existing fallback behavior.

---

## Phase 0 — Foundations (do first; everything depends on these)

### 0.1 Session intent

Dictation today assumes "record → clean → paste". Commands, assistant and
notes need different end handling.

- Add `SessionIntent` enum: `Dictate` | `Command` | `Assistant` | `Note`
  (serde snake_case; `dory`/`session` layer). Store the active intent next to
  `current_session_id` (`context.rs`) — a new `Arc<RwLock<SessionIntent>>`,
  initialized in both `bootstrap()` and `bootstrap_test()`.
- Extend `start_microphone_at`/the spawn helpers so a caller can pass an
  intent (default `Dictate`). `finish_stop` branches on it at output time
  (see per-phase specs below for each intent's behavior).
- `CaptureKind` stays for provenance; intent is orthogonal.

### 0.2 Multiple hotkeys

- `register_dictation_hotkey` currently unregisters everything then registers
  one combo (commands/mod.rs:87). Refactor to a `HotkeyRegistry` that maps
  shortcut → action (`dictate`, `command`, `assistant`, `note`, `cancel`,
  `mode:<id>`), each via `on_shortcut` with its own handler.
- The Windows LL hook supports **one** modifier-only combo. Extend
  `set_combo`/hook state to `Vec<HookState>` (one `HotkeyStateMachine` per
  combo), matched by `held_modifiers(&down) == st.required`. Non-Windows:
  modifier-only combos stay rejected (existing message).
- Reserve `Escape` handling: while a session is recording, the hook's `down`
  set already observes VK_ESCAPE — add an `on_cancel` callback that calls
  `session::cancel_owned` when recording is active. Escape must **pass
  through** (don't swallow the key). Also wire `api.cancelRecording` in the
  frontend (button on the recording UI + HUD hint text).
- New settings shape: `hotkeys: { dictation: string, command: string|null,
assistant: string|null, note: string|null, cancel: "Escape"|null }` —
  migrate `hotkey` → `hotkeys.dictation` in migrate_v4_to_v5; keep reading
  legacy `hotkey` patches in `apply_legacy_compute_patch`-style translation.

### 0.3 Output actions

- New `OutputAction` enum: `Paste` | `PasteEnter` | `Copy` | `Hud` |
  `AppendFile{path}` | `RunCommand{template}` (serde tagged). Add
  `output: OutputAction` to the upcoming Mode struct and a global default
  `output_action` + `send_key: "enter"|"shift_enter"|"ctrl_enter"` settings.
- `TextInjector::inject` gains a sibling `deliver(text, action, target_hwnd)`:
  - `Paste` = today's path.
  - `PasteEnter` = inject, then 120 ms later synthesize the configured send
    key (Windows: SendInput VK_RETURN; Linux: enigo `Key::Return`/`Key::Return
+Shift`; failure → `fallback_copy` path unchanged).
  - `Copy` = set clipboard (no restore-scheduling delete; leave text there),
    `pasted:false`, message "Copied".
  - `Hud` = emit result via `DoryEvent`, no clipboard touch.
  - `AppendFile` = append `text` + `\n` to path (create dirs; error →
    `InjectionFeedback` error message).
  - `RunCommand` = spawn `cmd /c`/`sh -c` with `{text}` substituted, detached,
    capture stderr→log; if template lacks `{text}`, pass text on stdin.
  - Keep `inject_text` IPC working (Paste semantics) — the LAN API
    `/v1/inject` depends on it.

### 0.4 Selected-text capture

- New `platform::capture_selection() -> Result<Option<String>, String>`:
  1. Snapshot clipboard text + sequence (`ClipboardAccess`).
  2. Synthesize Ctrl+C (Windows: reuse the `SendInput` plumbing in
     windows.rs — add a `simulate_copy()` sibling of `simulate_paste`; Linux:
     `simulate_copy_with_enigo`; macOS: `Err`/`None` → feature degrades).
  3. Poll clipboard sequence/text up to ~400 ms for a change.
  4. Read text, then restore the prior clipboard snapshot immediately (reuse
     `RestoreChain`/`snapshot`/`restore` — do NOT rely on the deferred 900 ms
     restore; selection capture must leave the clipboard untouched).
  5. Return `None` when nothing was selected (clipboard unchanged or empty).
- Guard: hold `CLIPBOARD_OPERATION` mutex for the whole sequence so a
  concurrent inject can't interleave.
- Tests: Windows code paths behind `#[cfg]`; unit-test the "unchanged →
  None" decision logic with a fake clock/injected reader.

### 0.5 Prompt plumbing for non-dictation tasks

- `rewrite/prompt.rs`: add `build_task_messages(system_preamble,
instruction, inputs: &[(&str,&str)] )` producing a message list shaped like
  `build_messages` (stable few-shot-free prefix + final user turn) for
  command/assistant/custom-mode calls. Keep `BASE_SYSTEM` untouched.
- `rewrite/client.rs`: add `FlowClient::complete(messages) -> Result<String>`
  sharing the tokenize/budget/stop-token/`extract_content` machinery but
  skipping `build_messages` and skipping `accept_rewrite` — callers apply
  `accept_task_output` (0.3 in ground rules) themselves. `rewrite()` stays
  as-is for cleanup.
- `FlowRuntime::ensure` already keys on the full `RuntimeRequest`; command/
  assistant calls reuse the same warm server — no new lifecycle work.

### 0.6 Mode data model (foundation for 1.3)

- New `Mode` struct in `settings/config.rs`:
  `{ id, name, icon?, hotkey: Option<String>, triggers:
{ apps: Vec<String>, spoken: Vec<String> }, language:
Option<String|"auto">, dictation_mode: DictationMode, cleanup_level,
intelligence_tier, style, custom_instructions: String (≤2000 chars,
empty=off), context: { selected_text: bool, clipboard: bool, window_title:
bool }, output: OutputAction, send_key, enabled }`.
- `AppSettings.modes: Vec<Mode>` + `default_mode_id` + `mode_triggers_enabled`.
  Ship four built-in modes on migration: Dictation (current behavior),
  Message (chat style + PasteEnter option off), Email, Coding
  (dictation_mode=coding, tier raw_verbatim).
- `AppSettings::mode_for_process(process) -> &Mode` + `resolve_mode(process)
-> Mode` (extends `for_application`'s basename matching; keep
  `application_profiles` working — a matching mode's app trigger wins over a
  legacy profile's style; profile dictionary terms still merge).
- Resolver order: explicit hotkey/intent > app trigger > default mode. Spoken
  triggers (e.g. "note mode") resolve on the first ~5 words of transcript —
  implement as a cheap prefix check in `postprocess_transcript` before
  formatting.

---

## Phase 1 — The five biggest wins

### 1.1 Snippets (spoken trigger → expands to stored text)

- `AppSettings.snippets: Vec<Snippet{id, trigger, expansion, enabled}>`;
  trigger ≤60 chars, expansion ≤4000 (match Wispr limits).
- Engine: `formatting/snippets.rs` — `apply_snippets(text, snippets)`,
  case-insensitive whole-phrase match on word boundaries, applied **inside**
  `format_transcript_ex` right after `custom_replacements.apply`
  (`formatting/mod.rs:157`), before normalizers/capitalization (expansions
  carry their own casing; don't re-clean them — match competitor behavior of
  inserting verbatim).
- Pass snippets through `FormatRequest` (new field) from `postprocess_transcript`.
- Snippet triggers double as ASR hotwords: extend `session_vocabulary`
  (session.rs:93) to append trigger phrases.
- UI: new **Snippets** settings page (list + add/edit form; whole-array writes
  like `dictionary_terms`). Default snippets: none (no email on file — we're
  offline; note this divergence from Wispr).
- Tests: trigger matching (word-boundary, case, multi-word, disabled),
  ordering vs replacements, vocabulary inclusion, page render/save.
- Acceptance: saying "insert my address" mid-sentence pastes the expansion;
  expansion survives LLM polish verbatim (snippets applied **after** Stage 2
  too — apply once on the final text if the LLM altered boundaries; simplest
  correct: apply on `final_text` post-glossary in `postprocess_transcript`,
  not inside `format_transcript_ex` — decide once, test both orders).

### 1.2 Command mode (voice-edit selected text)

- Intent `Command` + hotkey (default `Ctrl+Win+Alt`, non-modifier required on
  Linux/macOS — validate in `HotkeyPicker`).
- Flow on stop: `capture_selection()` → if selection exists, call
  `FlowClient::complete` with a task prompt: system = "You edit text exactly
  as instructed. Output only the result." user = instruction + `Text:` +
  selection + `Instruction:` + spoken command. Apply `accept_task_output`.
  No selection → fall back to dictating the instruction's result as normal
  text (Paste action) — document this.
- Built-in command phrases that map to canned instructions before the LLM:
  "make this shorter/longer/formal/casual", "fix grammar", "turn into
  bullets", "translate to <language>" (language name resolved via
  `asr/languages.rs` catalogue), "reply to this".
- Output: replace selection = `Paste` action into `dictation_target_hwnd`
  (selection is still there → paste overwrites it).
- HUD: distinct phase label ("Editing…"/"Edited"); reuse
  `InjectionFeedback.message`.
- History: store as normal entry with `application_process` +
  `processing_mode:"command"`; store selection-before in `smart_transcript`
  field? No — add `command_input TEXT` column in schema v5 for auditing,
  NULL otherwise.
- Tests: prompt shape (no context leaks into prefix), accept/reject paths,
  no-selection fallback, translation instruction resolution.

### 1.3 Custom modes + per-mode hotkeys (uses 0.2/0.3/0.6)

- Mode editor UI: new **Modes** settings page — list, enable toggle,
  duplicate, editor drawer (name, hotkey picker, app triggers list, language,
  tier/cleanup, style, custom instructions textarea, context toggles, output
  - send key). `Preview` button → `preview_cleanup`/`preview_tier_cleanup`
    extended to accept a mode's instructions.
- Per-mode hotkeys register through the HotkeyRegistry as `mode:<id>`;
  conflicts detected and reported in UI (`hotkey:error` event — surface it).
- `custom_instructions` nonempty → Stage-2 uses `build_task_messages` with
  instruction = custom text; `accept_task_output` instead of
  `accept_rewrite`. This makes "Jira ticket", "commit message", "medical
  note" modes possible — include those three as optional presets on the page.
- `dictation_mode:"coding"` Stage-2 skip (session.rs:171/739) becomes
  per-mode: `run_llm` AND NOT mode.dictation_mode==coding.
- Tests: resolver precedence, trigger matching, hotkey registry conflicts,
  instruction path reaches LLM (mock client), per-mode language.

### 1.4 Context awareness (uses 0.4 + 0.5)

- Mode `context` toggles wire real capture: `selected_text` →
  `capture_selection()` at record start (cache on session); `clipboard` →
  read-only `clipboard.text()` (no snapshot churn); `window_title` →
  `platform::active_window().0`.
- Assembly: `postprocess_transcript` gains a `SessionContext` param; when any
  toggle is on, inputs are appended to the LLM user turn as labeled sections
  (`Selected text:`, `Clipboard:`, `Window:`) — user turn only (KV rule).
  Each section truncated to `context_size`-aware budget (reuse
  `completion_budget`); skip empty sections entirely.
- Privacy rule: context is only read when the resolved mode opts in; log at
  `info!` when captured; never persist context to history except the
  `command_input` column (command mode only).
- Tests: section assembly/truncation, absence when toggles off, no-prefix-
  contamination.

### 1.5 Output destinations + auto-send (uses 0.3)

- Mode `output` + global `output_action` default plumbed into `finish_stop`'s
  injection branch: replace the direct `TextInjector::inject` call with
  `deliver(text, resolved_action, hwnd)`.
- `PasteEnter` send-key synthesized via the 0.3 implementation; per-app
  overrides later — keep a `send_key` per mode.
- `RunCommand` security: show a warning chip in the mode editor; refuse when
  `template` is empty; `{text}` substitution documented.
- UI: output picker inside mode editor + global default on a new **Output**
  section in General or its own page.
- Tests: action dispatch per mode, `Copy` leaves clipboard, `PasteEnter`
  emits send-key (mock platform fn), `Hud` doesn't touch clipboard,
  `AppendFile` writes.

## Phase 2 — The rest of the high-impact list

### 2.1 File transcription

- Rust: `audio/file_decode.rs` — WAV via `wav_or_pcm_to_f32` (move it from
  api/server.rs to `audio/`, re-export), other formats via `symphonia`
  (new dep — justify: hound is WAV-only); decode → downmix → `AudioResampler`
  → Vec<f32>. Reject files > 2 h; > N MB prompt-confirm.
- Transcribe: new `session::transcribe_samples(ctx, samples, language) ->
Vec<{text, offset_ms}>` — feed audio to ASR in our own ≤30 s
  silence-aligned chunks (reuse the sidecar's algorithm: min-energy cut in
  last 5 s — port `segment_audio` to Rust so WE know the offsets) calling
  start_stream→push_audio→stop_stream per chunk; offsets give real segment
  timestamps for SRT/VTT.
- IPC `transcribe_file(path) -> {job_id}`; progress events
  `transcribe:progress {job_id, done_s, total_s}` via Dory; cancel command.
  Store result as a history entry (`application_process="file:<name>"`,
  `command_input`=path? no — add `source` column value `file`) + optional
  audio retained under the same audio_retention rules (save the decoded PCM).
- Export: write `.txt` (plain) / `.srt` / `.vtt` next to Downloads via
  `export_audio`-style pathing; timestamps = segment offsets.
- UI: drop zone / file picker on a new section in DictateHome (or History
  toolbar button); progress inline; result opens its history entry.
- Tests: WAV decode parity with `wav_or_pcm_to_f32`, segmentation offsets,
  cancel mid-file, SRT formatting, retention integration.

### 2.2 Local voice assistant

- Intent `Assistant` + hotkey (default unset). On stop: run the transcript
  through `build_task_messages` — system: "You are a concise assistant.
  Answer in ≤6 sentences." + optional context sections (selected/clipboard/
  window per mode toggles); deliver per mode output defaulting to `Hud`.
- HUD needs to show multi-line answers: extend overlay geometry
  (`overlay_dims` kind → e.g. 320×160 "response" kind) + Overlay component
  response bubble (scrollable, markdown-lite) + `assistant:response` event.
  Keep 1200 ms auto-hide for normal phases; response stays until dismissed
  or next dictation.
- History entry with `processing_mode:"assistant"`.
- Tests: prompt contents (no dictation few-shot leakage), HUD event, history
  write, follow-up=none (stateless — keep it simple; no conversation memory
  in v1).

### 2.3 Notes / scratchpad

- Intent `Note` + hotkey + tray item "New note". Output: saved to history
  with `kind="note"` (new column in schema v5: `kind TEXT DEFAULT
'dictation'` covering dictation/note/command/assistant/file) and shown in
  a new **Notes** top-level view (`NavTab` + `Navigation` ITEMS + App.tsx
  branch): searchable list (reuse `query_history` filtering by kind), pin,
  delete, copy, export-to-Markdown-folder button (`notes_folder` setting,
  default `~/Documents/Reflow Notes`, append `## <timestamp>\n\n<text>`).
- Notes view doubles as the "scratchpad": manual text field that appends a
  note without dictating.
- Tests: kind filtering in `HistoryStore::query_entries` (extend
  `HistoryQuery{kind}`), markdown export, view render.

### 2.4 Output translation

- Mode-level `translate_to: Option<lang>` OR instruction "translate to X".
  Implementation: task prompt "Translate the following to <language
  name>. Output only the translation." — needs the "never translate" rule
  kept OUT of this path (it's in `BASE_SYSTEM`, which task prompts don't
  share — verify by test).
- Optional: expose Qwen3-ASR's own translation for en→X during ASR? Only if
  the sidecar supports a translate flag; otherwise LLM path only. Check
  `qwen3_asr_runtime.py` first; prefer LLM path for uniformity.
- Tests: translation goes through task path, dictation path still refuses
  translation (existing prompt test stays green).

### 2.5 Learn from corrections

- HistoryView: allow editing `final_transcript` (pencil → textarea → Save →
  `update_transcript` exists already).
- On save: diff edited vs prior final (word-level); for each user-corrected
  token (changed words that differ only in casing/spelling/hyphenation),
  emit a **dictionary suggestion**: new `dictionary_suggestions` surfaced via
  `DoryEvent`/IPC → toast + a "Suggestions" section on DictionaryPage
  (Accept → `save_dictionary_term`; Dismiss → persisted so it doesn't
  re-ask — store dismissed pairs in settings JSON).
- Tests: diff→suggestion extraction (pairs, frequency, casing), accept/
  dismiss persistence, no suggestion for rewrites (only real token swaps).

## Phase 3 — Nice-to-haves

Implement each as independently as possible; each needs its own tests.

- **3.1 Usage stats:** `history/stats.rs` — SQL aggregates: total words,
  chars, dictations, minutes spoken, per-app breakdown, per-day series (30d),
  streak days, estimated time saved (`words / 40 wpm`). IPC `get_stats`.
  New **Stats** view or an Advanced-page section (recommend section — fewer
  nav items); a small sparkline via existing CSS, no chart dep.
- **3.2 Sound cues:** bundle two tiny wavs (`assets/start.wav`,
  `assets/stop.wav`, generate with `scripts/` tone writer). Play from the
  _frontend_: emit `hud:sound {kind}` on state transitions → OverlayApp
  `new Audio()` playback. Zero Rust audio deps. Settings: `sounds_enabled`
  (default on), `sounds_volume` 0-1.
- **3.3 Escape-to-cancel + re-paste last:** covered by 0.2 (Escape) plus
  `repaste_last` command reusing last final_transcript → `TextInjector`.
  Wire `app:auto-stop` event to a toast while at it (currently unheard).
- **3.4 History pins/tags/filters:** schema v5 columns `pinned INTEGER
DEFAULT 0`, `tags TEXT` (comma-joined). `HistoryQuery` gains
  `{pinned_only, tag, kind}`. UI: pin toggle + tag chips + app/kind filter
  chips on HistoryView.
- **3.5 Retry with options:** `retry_history_transcript` gains optional
  `{tier?, mode?, language?}` overrides; UI: "Retry" submenu on the row
  menu.
- **3.6 Import/export config:** `export_config(path)`/
  `import_config(path)` — JSON bundle `{settings, snippets, modes,
dictionary_terms, custom_replacements}` (exclude devices/tokens/paths that
  don't transfer; validate versions; merge vs replace toggle in UI).
  Buttons on AdvancedPage.
- **3.7 Encrypted history at rest:** optional (`history_encryption:
bool`). Key: `keyring` → OS credential store ("reflow/history-key");
  first enable generates 256-bit key. Encryption: AES-256-GCM
  (`aes-gcm`) applied at the `insert_entry`/read boundary for
  `raw_transcript|smart_transcript|final_transcript|command_input` +
  `history_audio.pcm` (nonce prepended to ciphertext blob). Migration
  encrypts existing rows in a transaction; recovery path untouched. Plain
  FTS/`instr` search can't work on ciphertext — search when encrypted =
  decrypt-and-scan (bounded pages), documented tradeoff; default OFF.
- **3.8 Privacy exclusions:** `excluded_apps: Vec<String>` — at
  `start_microphone_at`, if `active_window().1` matches (same basename
  normalization as `for_application`) refuse with
  "Recording paused in <app>" `InjectionFeedback` + skip audio capture
  entirely (do NOT record-then-delete). Optional `mute_in_games`? No —
  YAGNI.
- **3.9 Media duck:** `duck_media` setting (default off, Windows-only v1):
  enumerate `IAudioSessionManager2` sessions via the `windows` crate (add
  `Win32_Media_Audio` + related features), set non-owning sessions to 50 %
  (`ISimpleAudioVolume`) on start, restore on stop. Linux/macOS: no-op with a
  tooltip. Deliberately NOT MediaPlayPause toggling (false-positive resumes).
- **3.10 Mic auto-switch:** `follow_default_mic` (default on): when
  `microphone_device_id` unset, re-resolve the default device at each
  `start_microphone_at` (already the behavior) AND poll every 5 s while idle
  to re-resolve if the default changed mid-session-only errors → current
  behavior unchanged. True mid-recording failover is out of scope (note it).
- **3.11 Voice punctuation commands:** extend `PunctuationInferer`/
  replacements with phrases: "new paragraph" → `\n\n`, "open bracket"…,
  "smiley"→`:)`. And dictation commands recognized verbatim at
  `postprocess_transcript` top when the whole utterance matches: "scratch
  that" → cancel injection + delete the pending history row; "undo that" →
  `undo_last_ai_edit_inner` then re-paste. Gate behind
  `voice_commands_enabled`.
- **3.12 Local automation API:** when `api_enabled && api_bind=="localhost"`,
  the existing axum server already serves on 127.0.0.1 — expose a
  long-lived "automation token" (Settings → Advanced shows it, stored
  sha256 like pairing tokens but `permissions:{stream,history,injection}` all
  true, revocable). Add REST `POST /v1/dictate {text?}` (types text) and
  `POST /v1/session {action:start|stop|cancel}` mapping to the session
  functions. Document a `reflowctl`-style curl example in docs/. CLI: add
  `--dictate "text"` (runs `inject_text` in a headless main-path).
- **3.13 Offline update notice:** version label on AdvancedPage
  (`tauri::config` version) + "Check releases" button that just opens the
  GitHub releases URL in the browser — deliberately no auto-check.
- **3.14 Portable mode:** `--portable` flag in main.rs → data dir =
  `<exe dir>/reflow-data` (touch `platform` path resolution — find where
  `dirs::data_dir` is called for `app_dir`; centralize if needed). Document;
  no UI.
- **3.15 Min-duration discard:** `min_dictation_ms` (default 300): if a
  PTT release lands under it and hands-free didn't engage, cancel silently
  (no transcribe, no history row).
- **3.16 Airplane mode (make `offline_mode` real):** when true → all
  download commands (`install_model`, `install_intelligence_model`,
  `install_llama_runtime`, `repair_runtime`, calibration downloads) fail fast
  with "Offline mode is on"; `reqwest` clients check the flag. Badge in
  TitleBar; toggle on AdvancedPage + tray. Audit: `grep -r "http"` for stray
  fetch/reqwest calls and confirm the only network surfaces are the pinned
  downloads + LAN API.
- **3.17 Network ledger:** log every outbound request (URL host, bytes,
  timestamp) to `<data_dir>/network-journal.jsonl`; render read-only list on
  AdvancedPage ("Every connection Reflow has made").
- **3.18 Offline model import:** `import_model_file(path)` — pick a
  `.gguf`/model dir from disk, verify sha256 against the manifest entry
  (checksums already in `runtime-pins.json`/manifests), copy into models dir,
  register installed. For air-gapped machines (pair with download on another
  PC + `export` instructions in docs).
- **3.19 Power profiles:** `platform::on_battery()` (Windows:
  `GetSystemPowerStatus` via windows crate; Linux: read
  `/sys/class/power_supply/*/status`; macOS: `iopm`/`pmset -g batt` or no-op)
  → `power_policy: { unload_on_battery: bool, battery_preset:
Option<Preset> }` in settings; on AC↔battery transition (poll 30 s) apply.
- **3.20 High-contrast HUD + announcements:** `hud_contrast: "standard"|
"high"` (token override set in globals.css), announce injection results via
  `aria-live` region in Overlay + toasts (mostly free — ensure
  `role="status"`).
- **3.21 Paste delay + type-instead-of-paste:** `paste_delay_ms`
  (0-2000, default 30 — current `thread::sleep(30)` becomes configurable),
  `inject_method: "paste"|"type"` where `type` uses enigo `text()` (Windows
  may keep SendInput Unicode; implement per-OS), for RDP/VMs that block
  paste. Warn it's slow >200 chars.
- **3.22 Trailing space/newline + context capitalization:**
  `append_space` bool; "auto-capitalize continuation" = peek at
  clipboard/selection? Skip peek — just `capitalize_first: bool` (default
  on, keep today's behavior).

## Phase 4 — Heavy features (last; each is its own milestone)

- **4.1 Meeting/system-audio mode:** new `CaptureKind::SystemMix`: WASAPI
  loopback via cpal (`default_output_device` + `default_output_config`,
  stream flag LOOPBACK is implicit for render devices in cpal 0.15 — verify
  on Windows; Linux: PipeWire monitor sources appear as input devices —
  enumerate with a `monitor` marker). Mix mic+system PCM (sum/2), then the
  normal path. Post: "Summarize" button → task prompt on the transcript
  (chunked summarization — reuse `rewrite_segments` splitting discipline).
  Big scope: gate behind `meeting_mode` setting; X11/Wayland pipewire quirks
  documented.
- **4.2 Speaker diarization for file transcription:** stretch goal — needs a
  diarization model in the Python sidecar (pyannote is heavy; investigate
  `onnx-community` local options or simple energy/embedding clustering).
  If no local option lands cleanly, ship file transcription without
  diarization and leave the column/API shape in place (`speakers:
Option<Vec<..>>`).
- **4.3 Two-stage assistant tools:** assistant can emit a recognized
  command line (`SEARCH: <query>` → open local browser URL; `NOTE: <text>` →
  save note). Keep a strict allowlist parser — never execute shell from LLM
  output.

## Suggested order inside phases

Phase 0: 0.1 → 0.3 → 0.5 → 0.4 → 0.2 → 0.6.
Phase 1: 1.1 → 1.5 → 1.2 → 1.4 → 1.3.
Then 2.x by listed order; 3.x may be parallelized across the module
boundaries noted; 4.x last, independently.

## Non-goals (explicitly do NOT build)

Cloud sync/accounts, team/shared snippets, any telemetry, auto-updater
daemons, screen-content OCR/screen context (competitors do it; it requires
accessibility-scoped screen capture — revisit later only if a local OCR story
exists), mid-recording mic failover, conversation memory for the assistant,
mobile-side feature changes (Android client untouched except tolerance for
new ServerMsg fields — keep protocol additive/backward-compatible).

## Global acceptance checklist

- `npm run check`, `cargo test`, `cargo clippy -D warnings`, python
  `--selftest` green.
- Airplane mode ON + network disconnected (hosts file blackhole):
  dictation, cleanup, commands, assistant, notes, stats, file transcription
  all still work; only downloads are blocked with clear errors.
- No new field lacks a serde default; settings v4 documents migrate
  forward; history DB v4 files migrate in place.
- Every new UI string routed through existing components; dark/light themes
  verified; `reduce_motion` respected for new animations.
- README features section updated; `docs/` gets a new verification record
  following `release-verification-0.2.0.md` conventions.
