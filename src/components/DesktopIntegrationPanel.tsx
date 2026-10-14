import { useState, type ReactNode } from "react";
import { useDesktopIntegration } from "../hooks/useDesktopIntegration";
import { api } from "../services/tauriApi";
import type { DesktopIntegrationStatus } from "../types";

interface ShortcutControls {
  status: DesktopIntegrationStatus | null;
  busy: boolean;
  configure: () => void;
  setShortcut: (action: string, shortcut: string | null) => void;
}

export function DesktopIntegrationPanel({
  renderShortcuts,
}: {
  renderShortcuts?: (controls: ShortcutControls) => ReactNode;
} = {}) {
  const { status, error: readError, acceptStatus } = useDesktopIntegration();
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const perform = async (action: string, call: () => Promise<void>) => {
    setBusy(action);
    setError(null);
    try {
      await call();
    } catch (failure) {
      setError(`${action}: ${String(failure)}`);
    } finally {
      setBusy(null);
    }
  };
  const configure = () => {
    void perform("Could not open desktop shortcuts", api.configureGlobalShortcuts);
  };
  const setShortcut = (action: string, shortcut: string | null) => {
    void perform("Could not change shortcut", async () => {
      acceptStatus(await api.setDesktopShortcut(action, shortcut));
    });
  };
  const shortcuts = renderShortcuts?.({ status, busy: busy !== null, configure, setShortcut });
  if (readError)
    return (
      <>
        {shortcuts}
        <p role="alert" className="text-sm text-danger">
          {readError}
        </p>
      </>
    );
  if (!status || status.session === "web") return <>{shortcuts}</>;
  const wayland = status.session === "wayland";
  const keyboardReady = status.keyboard.state === "ready";
  const keyboardPending = status.keyboard.state === "pending";
  const retryKeyboard =
    status.keyboard.state === "denied" || status.keyboard.state === "unavailable";
  let keyboardButtonLabel = "Enable automatic paste";
  if (keyboardPending) keyboardButtonLabel = "Waiting for keyboard permission…";
  else if (retryKeyboard) keyboardButtonLabel = "Retry keyboard access";
  return (
    <>
      {shortcuts}
      <div className="space-y-3 text-xs text-muted leading-relaxed">
        <p role="status">
          {status.shortcut.state === "ready"
            ? `Dictation shortcut ready: ${status.shortcut.assigned_trigger ?? "assigned"}.`
            : (status.shortcut.message ?? "Checking desktop shortcuts…")}
        </p>
        {wayland && (
          <>
            <div className="flex flex-wrap gap-2">
              <button
                type="button"
                className="btn btn-ghost"
                disabled={busy !== null}
                onClick={configure}
              >
                {busy === "Could not open desktop shortcuts"
                  ? "Opening desktop shortcuts…"
                  : "Configure desktop shortcuts"}
              </button>
              {status.shortcut.state === "unavailable" && (
                <button
                  type="button"
                  className="btn btn-ghost"
                  disabled={busy !== null}
                  onClick={() => {
                    void perform("Could not reconnect shortcuts", api.retryDesktopShortcuts);
                  }}
                >
                  Reconnect shortcuts
                </button>
              )}
            </div>
            <p role="status">
              {keyboardReady
                ? "Keyboard access ready for automatic paste."
                : (status.keyboard.message ??
                  "Keyboard access is needed for automatic paste. Your transcript stays available for manual paste.")}
            </p>
            {!keyboardReady && (
              <button
                type="button"
                className="btn btn-ghost"
                disabled={busy !== null || keyboardPending}
                onClick={() => {
                  void perform("Could not request keyboard access", api.requestKeyboardAccess);
                }}
              >
                {keyboardButtonLabel}
              </button>
            )}
            <p>
              {status.assigned_shortcuts.cancel
                ? `Cancel shortcut: ${status.assigned_shortcuts.cancel}.`
                : status.shortcut_editable
                  ? "No global cancel key is assigned. Choose one above, or use Cancel in Reflow."
                  : "No global cancel key is assigned. Use Cancel in Reflow, or assign Cancel in desktop shortcuts."}
            </p>
            {!status.hud_supported && (
              <p>
                This desktop cannot place the floating HUD. Recording status remains available in
                Reflow.
              </p>
            )}
            {!status.foreground_app_available && (
              <p>
                Foreground app details are unavailable on this desktop. For terminals, choose Ctrl +
                Shift + V in Output → Paste shortcut.
              </p>
            )}
          </>
        )}
        {error && (
          <p role="alert" className="text-sm text-danger">
            {error}
          </p>
        )}
      </div>
    </>
  );
}
