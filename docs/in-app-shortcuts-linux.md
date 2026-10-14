# In-app shortcut editing on KDE Wayland

14 October 2026. This extends the smoothness refactor after the user reported that KDE's shortcut settings were too difficult to navigate.

Open **Settings → General**, click the current dictation key and press the desired combination. Reflow applies the actual KDE binding and displays it after native readback confirms it. Enabled command, assistant, note and cancel shortcuts use the same editor. Cancel's picker can capture bare Escape; modified Escape also works in other pickers. A newly enabled action can still require the desktop's initial approval.

KDE persists these assignments. Reflow's saved hotkey values remain the initial portal requests; they are not used to overwrite the desktop's actual binding. Conflicts are checked against actual desktop keys, including other Reflow actions, rather than stale preferred values. Editing does not tear down/rebind the portal session or add a second keyboard listener. The portal continues delivering press/release events.

Other Wayland desktops retain **Configure desktop shortcuts**, and it remains a fallback for key types the in-app KDE adapter does not translate. Windows/X11 continue using the existing settings and registration path. No Qt runtime or development package was added to Reflow.

## Implementation

The adapter uses KDE's `allActionsForComponent`, `shortcutKeys`, `globalShortcutAvailable` and `setForeignShortcutKeys` APIs. It only edits an existing action under the application's configured identifier. The command validates action membership, requires an active portal session, rejects changes during dictation and holds recording admission while applying the key. A disconnect during application is reported as requiring reconnection rather than falsely claiming the key is active. It confirms the resulting native key before publishing success. UI errors preserve the previously confirmed display; the returned confirmed snapshot also updates the page without depending solely on event delivery.

The Qt key/modifier mapping follows Qt's published constants. Every serialized `QKeySequence` is a structure containing an array of exactly four signed integers, with unused entries zero-padded. Keeping the correct array length matters for older KDE decoders.

## Verification and incident

Frontend lint, format, types and all 252 tests pass. The Rust all-target suite passes 866 tests with 19 opt-in/environment tests ignored; Clippy with warnings denied and the optimized Tauri build pass. The targeted native test passes against an isolated KF6 `kglobalacceld`: actual assignment/readback, clearing, conflicting-action rejection, unknown-action rejection and cleanup. Run it with:

```bash
bash scripts/test_kde_shortcuts.sh
```

The script creates a private D-Bus session and private configuration/cache/runtime directories before starting the bus. It verifies the service is owned by its own offscreen daemon process. It never sends test key sequences to the desktop compositor. The opt-in Rust test refuses to run without the isolation marker. Native window/key delivery still needs a user test in the optimized app.

The initial native test used a one-entry sequence, and the installed KDE decoder aborted while reading it. KWin automatically restarted, interrupting desktop applications. Reflow was restarted. The failing operation was a read-only availability query; no user key was changed. The sequence format was corrected, a four-entry contract regression was added, and all subsequent protocol tests were moved to the private service. The incident and raw log locations are retained in [in-app-shortcuts-linux-results.json](in-app-shortcuts-linux-results.json).

A SWE-2 Max review was requested through Devin CLI. It returned no output after more than ten minutes and was stopped; no review result from that run is claimed. The primary agent reviewed and tested this change.

## References

The [GlobalShortcuts portal](https://flatpak.github.io/xdg-desktop-portal/docs/doc-org.freedesktop.portal.GlobalShortcuts.html) offers desktop configuration rather than a portable direct key setter. The KDE adapter follows the [KGlobalAccel D-Bus interface](https://github.com/KDE/kglobalaccel/blob/master/src/org.kde.KGlobalAccel.xml), its [configuration notification implementation](https://github.com/KDE/kglobalacceld/blob/master/src/kglobalacceld.cpp), and the [QKeySequence wire representation](https://github.com/KDE/kglobalaccel/blob/master/src/kglobalshortcutinfo_dbus.cpp). Key values come from [Qt's public key/modifier definitions](https://github.com/qt/qtbase/blob/dev/src/corelib/global/qnamespace.h).

## Final observations

The final optimized build passes. A manually started development instance is currently active and includes the final source changes; a release launch focuses that existing instance through the single-instance guard. KDE's live API and configuration file now both show `Ctrl+Space`. After that assignment appeared, a real 3.712-second dictation recognized 25 characters and dispatched insertion successfully without manual-copy fallback (967 ms release-to-insertion dispatch). Visible insertion and the ease of the in-app workflow remain user-confirmed acceptance items.

One parallel frontend run failed an existing Notes pagination/delete assertion during compilation load. The full unchanged suite then passed all 252 tests with two workers. No Notes code or test timeout was changed. Logs remain until explicit confirmation of smoothness and functionality.
