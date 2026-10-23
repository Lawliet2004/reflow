import { useEffect, useRef, useState } from "react";
import { api, safeListen } from "../services/tauriApi";

/** Uses the native binding without admitting an ASR session or recording audio. */
export function ShortcutProbe({ onSuccess }: { onSuccess: () => void }) {
  const [listening, setListening] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const attempt = useRef(0);
  const release = useRef<() => void>(() => {});
  useEffect(
    () => () => {
      attempt.current++;
      release.current();
    },
    [],
  );

  const start = async () => {
    const current = ++attempt.current;
    release.current();
    setListening(true);
    setError(null);
    let id: number | undefined;
    let unsubscribe: (() => void) | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let heard = false;
    const stop = () => {
      clearTimeout(timer);
      unsubscribe?.();
      if (id !== undefined) void api.endShortcutCapture(id).catch(() => {});
      if (current === attempt.current) {
        attempt.current++;
        setListening(false);
      }
    };
    release.current = stop;
    try {
      unsubscribe = await safeListen<{
        id: number;
        action: string;
        pressed: boolean;
      }>("hotkey:capture", (event) => {
        if (current !== attempt.current || event.id !== id) return;
        if (!["dictate", "dictation"].includes(event.action)) return;
        if (event.pressed && !heard) {
          heard = true;
          onSuccess();
          clearTimeout(timer);
          // Retain capture until release; portal toggle bindings have no key-up.
          timer = setTimeout(stop, 2_000);
        } else if (!event.pressed && heard) stop();
      });
      if (current !== attempt.current) {
        stop();
        return;
      }
      id = await api.beginShortcutCapture();
      if (current !== attempt.current) {
        stop();
        return;
      }
      if (!id) throw new Error("Shortcut tests require the desktop app.");
      timer = setTimeout(() => {
        stop();
        setError(
          "No dictation shortcut received. Check its assignment, then try again, or use the Record button.",
        );
      }, 30_000);
    } catch (failure) {
      stop();
      if (current === attempt.current) setError(String(failure));
    }
  };
  return (
    <div className="space-y-2" aria-live="polite">
      <button
        type="button"
        className="btn btn-secondary"
        disabled={listening}
        onClick={() => {
          void start();
        }}
      >
        {listening ? "Press your dictation shortcut…" : "Test shortcut"}
      </button>
      {listening && (
        <button type="button" className="btn btn-ghost" onClick={() => release.current()}>
          Stop shortcut test
        </button>
      )}
      {error && (
        <p role="alert" className="text-sm text-danger">
          {error}
        </p>
      )}
    </div>
  );
}
