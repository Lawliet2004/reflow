History storage
===============

Encryption is off by default. `set_encryption(true)` obtains a 256-bit key from
the OS credential entry with service `reflow` and user `history-key`. Windows
uses Credential Manager, macOS uses Keychain, and Linux uses persistent Secret
Service storage. An unavailable credential store is an error; there is no
plaintext key-file or mock-store fallback on supported platforms.

AES-256-GCM protects raw, smart, and final transcripts, command selections,
retained recording PCM, and complete file subtitle JSON. Each value has a fresh
96-bit nonce. Associated data binds ciphertext to its history row and column.
Application names/processes, dates, counts, kinds, pins, and tags remain plaintext
so filters and usage aggregates work without scanning transcript text. Exported
text/audio files are ordinary user-readable files, not encrypted backups.

Schema 6 persists encryption state and an authenticated key verifier. Opening an
encrypted database requires its existing key even when preferences currently say
encryption is off. Key verification happens before schema migrations. Missing or
wrong keys preserve the original; recovery reuses a separate database across
restarts (including the newest UUID recovery store from older releases) and refuses
to create a replacement encryption key. The recovery notice explicitly states
that recovery history is unencrypted; the preference alone must not be presented
as proof of successful encryption when a recovery notice is active. If encryption
is requested but unavailable, private writes (including retry/edit/audio/subtitles)
are blocked until the key is restored or the user explicitly disables encryption.
Switching encryption on or off rewrites
all sensitive tables and the manifest in one transaction. Any authentication or
SQLite error rolls the operation back. The OS key is retained when disabling so
encrypted backups remain usable.

Automatic transcript retention, including disabling history, preserves explicitly
saved manual and voice notes. Explicit clear-history and delete operations still
remove the selected notes. Audio retention remains independent. File imports
require history to be enabled and recheck that choice immediately before saving.

Editing or retrying a file transcript invalidates its old subtitle text and timing
in the same transaction as the transcript change. TXT exports use the current
transcript; SRT/VTT require importing the audio again to regenerate timing data.

Before a mode change, old WAL frames are checkpointed and truncated and the
connection switches to DELETE journaling. If another reader prevents this, the
mode change fails before rewriting records; old plaintext WAL frames must not
survive a successful enable.

Encrypted text search decrypts metadata-filtered pages of 100 records in memory,
up to 10,000 candidate records. Larger searches return an error asking for date,
application, or kind filters; they do not return a misleading partial total.
Matching uses literal Unicode lowercase substring search. This has a higher cost
than SQLite substring search. Empty searches and metadata filters remain SQL.

Usage statistics count live dictation records (`source = dictation` and positive
duration), including spoken notes, and exclude manual notes and file imports.
The chart covers 30 local calendar days and includes zero-activity dates. The
streak ends today, or yesterday until today's first dictation. Estimated time
saved uses the prompt's 40 words/minute typing baseline.

Implementation references:

- https://docs.rs/aes-gcm/0.10.3/aes_gcm/
- https://docs.rs/keyring/3.6.3/keyring/

Tests inject `HistoryKeyProvider` instances and never write OS credentials.
