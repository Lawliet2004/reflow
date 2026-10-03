# Local automation and portable mode

Enable the API in Settings and select **localhost** binding. In Advanced, create
an automation token and copy the displayed value. Reflow displays the token once
and stores only its SHA-256 hash. Creating another token replaces the previous
one. Revoke it from the same panel when no longer needed. Automation tokens grant
stream, history and text insertion access; keep them out of scripts committed to
source control.

The API uses the existing HTTPS listener, including on localhost. Obtain its
public certificate from your local `config/lan-identity.json` file: the
`certificate_pem` field is public; do not copy `private_key_pem`. Save that
certificate as `reflow-local.pem` and pass it to curl with `--cacert`. The API
port is configurable; the examples below use 7840. Set `REFLOW_TOKEN` in your
current shell to the copied token.

```sh
curl --cacert reflow-local.pem https://127.0.0.1:7840/v1/dictate \
  -H "Authorization: Bearer $REFLOW_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"text":"This text is pasted into the foreground application."}'

curl --cacert reflow-local.pem https://127.0.0.1:7840/v1/session \
  -H "Authorization: Bearer $REFLOW_TOKEN" \
  -H 'Content-Type: application/json' -d '{"action":"start"}'
# Speak, then stop and insert the transcript:
curl --cacert reflow-local.pem https://127.0.0.1:7840/v1/session \
  -H "Authorization: Bearer $REFLOW_TOKEN" \
  -H 'Content-Type: application/json' -d '{"action":"stop"}'
# Use {"action":"cancel"} to discard this token's recording instead.
```

`/v1/dictate` requires nonempty text, up to 1 MiB, and returns `pasted`,
`fallback_copy` and `paste_chord`. It honors clipboard restoration and checks
that the captured foreground target still has focus. Focus the intended editor
before calling it. An empty request is rejected; microphone dictation uses
`/v1/session`.

`/v1/session` accepts exactly `start`, `stop` or `cancel`. Its start response
includes `session_id`. Only the token that started the microphone session can
stop or cancel it. A later physical hotkey or phone recording is protected from
stale automation requests. Busy, expired and unowned sessions return HTTP 409;
invalid credentials return 401. The local automation routes reject paired phone
credentials even when phone permissions allow insertion.

Changing binding to LAN or disabling the API immediately disables automation
credentials on **all** API routes. The same automation token works again after
returning to localhost, unless it was revoked. Phone pairing remains separate.
No API request can create, reveal or revoke an automation token.

For direct headless text insertion, use:

```text
reflow --dictate "Text to paste"
reflow --portable --dictate "Text to paste"
reflow --portable --status
```

`--dictate` uses the current foreground editor and the configured clipboard
restore setting without opening the main window or starting ASR. A successful
paste keeps the process alive briefly for delayed clipboard restoration.
Failed paste attempts leave text copied and report the fallback.

`--portable` selects `<executable directory>/reflow-data` before any app
bootstrap. Settings, SQLite history, pairing credentials, TLS identity,
installed models and runtimes, logs, measurements and application caches use
that directory. Desktop WebView storage also follows the portable directory on
Windows and Linux. macOS WebView storage remains in the OS-managed WebKit data
store because WebKit does not support a custom data directory.

Portable mode requires write access beside the executable. Use the flag every
time you launch that copy, and quit an already running instance before
changing data directories. User-selected export destinations retain their own
paths. Portable mode does not transfer system startup registrations or OS
credential-store encryption keys; move encrypted history with its original key
or export it first.
