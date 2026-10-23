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
  compact = false,
  onShortcutSaved,
}: {
  renderShortcuts?: (controls: ShortcutControls) => ReactNode;
  compact?: boolean;
  onShortcutSaved?: () => void;
} = {}) {
  const { status, error: readError, acceptStatus } = useDesktopIntegration();
  const [failure, setFailure] = useState<{
    message: string;
    status: DesktopIntegrationStatus | null;
  } | null>(null);
  // A later desktop confirmation supersedes errors from an older assignment.
  const error = failure?.status === status ? failure?.message : null;
  const setError = (message: string | null) => {
    setFailure(message ? { message, status } : null);
  };
  const [busy, setBusy] = useState<string | null>(null);
  const perform = async (action: string, call: () => Promise<void>) => {
    setBusy(action);
    setError(null);
    try {
      await call();
    } catch (failure) {
      const message = String(failure);
      setError(
        compact
          ? /already used|assigned to both/i.test(message)
            ? "That shortcut is in use. Try another combination."
            : /invalid.*(shortcut|hotkey)|non-modifier|modifier-only|include a letter/i.test(
                  message,
                )
              ? "Include a letter, number or Space, such as Ctrl+Space."
              : /permission|approve|reconnect|not registered/i.test(message)
                ? "Allow shortcuts in the desktop prompt, then try again."
                : "That didn’t work. Try again or continue using Record in Reflow."
          : `${action}: ${message}`,
      );
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
      onShortcutSaved?.();
    });
  };
  const shortcuts = renderShortcuts?.({ status, busy: busy !== null, configure, setShortcut });
  if (readError)
    return (
      <>
        {shortcuts}
        <p role="alert" className="text-sm text-danger">
          {compact ? "Couldn’t check shortcuts. You can still use Record in Reflow." : readError}
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
  if (compact) {
    const shortcutReady = status.shortcut.state === "ready";
    const pending = status.shortcut.state === "pending";
    return (
      <>
        {shortcuts}
        {!shortcutReady && (
          <div className="space-y-2">
            <p role="status" className="text-sm text-muted">
              {pending
                ? "Allow shortcuts in the desktop prompt."
                : status.shortcut.state === "unassigned"
                  ? "Choose a shortcut above."
                  : "Shortcuts aren’t connected. You can still use Record in Reflow."}
            </p>
            {wayland && !pending && status.shortcut.state !== "unassigned" && (
              <button
                type="button"
                className="btn btn-secondary"
                disabled={busy !== null}
                onClick={() => {
                  void perform("Could not reconnect shortcuts", api.retryDesktopShortcuts);
                }}
              >
                {busy ? "Connecting…" : "Connect shortcuts"}
              </button>
            )}
          </div>
        )}
        {wayland && !keyboardReady && (
          <div className="flex flex-wrap items-center justify-between gap-3 border-t border-line pt-4">
            <div>
              <p className="text-sm text-ink">Paste into other apps</p>
              <p className="text-xs text-muted" role="status">
                {keyboardPending
                  ? "Allow access in the desktop prompt."
                  : "Optional. You can also copy your text."}
              </p>
            </div>
            <button
              type="button"
              className="btn btn-secondary"
              disabled={busy !== null || keyboardPending}
              onClick={() => {
                void perform("Could not request keyboard access", api.requestKeyboardAccess);
              }}
            >
              {keyboardPending ? "Waiting…" : retryKeyboard ? "Try again" : "Enable"}
            </button>
          </div>
        )}
        {error && (
          <p role="alert" className="text-sm text-danger">
            {error}
          </p>
        )}
        {wayland && (
          <details className="border-t border-line pt-3 text-xs text-muted">
            <summary className="cursor-pointer">More options</summary>
            <div className="mt-3 space-y-3">
              <button
                type="button"
                className="btn btn-ghost"
                disabled={busy !== null}
                onClick={configure}
              >
                Open desktop shortcuts
              </button>
              <p>Cancel a recording with the Cancel button in Reflow.</p>
              {!status.foreground_app_available && (
                <p>For terminals, choose Ctrl+Shift+V in Settings → Output.</p>
              )}
              {!status.hud_supported && <p>Recording status is shown in Reflow on this desktop.</p>}
            </div>
          </details>
        )}
      </>
    );
  }
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
