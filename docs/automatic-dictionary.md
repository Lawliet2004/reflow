# Automatic dictionary learning

Reflow learns spelling preferences locally when a user corrects dictated text.
It does not need cloud inference, an extra LLM, or model retraining.

- **Dictionary → Learn from corrections** is enabled by default. Turning it off
  stops new learning, including observation of external text fields. Saved
  vocabulary and correction rules continue to apply in non-verbatim cleanup.
- Saving an edit to a dictation in Reflow History automatically saves suitable
  corrections, without an Accept step. Manual notes and broad rewrites do not
  become dictionary rules. Existing suggestions remain available for review.
- On Windows, a successful local microphone dictation using the Paste action
  starts a background UI Automation observer. It waits for the pasted text to
  appear, watches only that focused field for up to two minutes, and waits for
  1.5 seconds without text changes before learning. A completed edit is also
  captured on leaving the field if its text is still available. Starting another dictation,
  changing focus, disabling learning, or excluding the app ends observation.
- Password fields, read-only controls, oversized fields, ambiguous repeated
  pasted text, and changes outside the pasted span are ignored. Copy, HUD,
  command, assistant, meeting, remote API, and Paste-and-send output do not start
  observation. Unsupported providers degrade to ordinary dictation.
- Small spelling, capitalization, hyphen, and word-joining/splitting changes
  are learned. Added prose, punctuation-only changes, and substantial rewrites
  are not. Arbitrary phonetic errors and semantic ambiguity are not guessed.
- Observed edits update the matching saved History entry and
  notify History and Home's recent dictations; raw ASR text is preserved. Late observations cannot replace a
  newer saved edit or recreate deleted history. Learning also works with history
  disabled, without creating a transcript record.
- Preferred spellings are recognition hints and exact text corrections, with
  literal replacement text and longer phrases taking priority. Qwen receives
  recognition hints; the current Phonon decoder only gets text correction after
  recognition. Raw cleanup deliberately preserves ASR text.
- Dictionaries use the current user's local settings directory (normally the
  OS account's application data). Portable installations use their selected
  data directory; sharing that directory shares its preferences. There are no
  cloud accounts. Deleting a dictionary rule suppresses automatic relearning of
  that same correction. A new correction updates the user's previous preference.

External-field observation currently supports Windows only and depends on the
target app exposing editable text through UI Automation. Reflow does not enable
accessibility modes in other applications. History-based learning is available
on all supported desktop platforms.

If an app clears or submits a field before a correction is observed, that edit
cannot be recovered. Observation begins after insertion; edits made before the
pasted span can be located are not attributed to the dictation.

Windows COM objects remain on a dedicated MTA thread with provider timeouts:
https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-threading

Text access uses UI Automation TextPattern or ValuePattern:
https://learn.microsoft.com/en-us/windows/win32/winauto/uiauto-ui-automation-textpattern-overview
