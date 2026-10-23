import React, { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Keyboard } from "lucide-react";
import { api, safeListen } from "../services/tauriApi";

interface HotkeyPickerProps {
  value: string;
  onChange: (value: string) => void;
  size?: "sm" | "md";
  /** Override for previews/tests. Native Windows is the only modifier-hook backend. */
  allowModifierOnly?: boolean;
  disabled?: boolean;
  ariaLabel?: string;
  allowEscape?: boolean;
}

const ORDER = ["Ctrl", "Alt", "Shift", "Win"];

/** KeyboardEvent.key → display modifier name, for keyup detection. */
const MODIFIER_KEYS: Record<string, string> = {
  Control: "Ctrl",
  Alt: "Alt",
  Shift: "Shift",
  Meta: "Win",
};

/**
 * Click to arm, then press a key/combo. Esc cancels.
 * Modifier-only combos (e.g. Shift+Win) are detected when ≥2 modifiers
 * are held simultaneously and then released. Waiting for release also allows
 * shortcuts such as Ctrl+Shift+K to include a non-modifier key.
 */
export const HotkeyPicker: React.FC<HotkeyPickerProps> = ({
  value,
  onChange,
  size = "md",
  allowModifierOnly = /Windows|Win32|Win64/.test(navigator.userAgent + navigator.platform),
  disabled = false,
  ariaLabel,
  allowEscape = false,
}) => {
  const heldRef = useRef<string[]>([]);
  const [recording, setRecording] = useState(false);
  const [needsKey, setNeedsKey] = useState(false);
  const [preparing, setPreparing] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const leaseRef = useRef<number | null>(null);
  const unsubscribeRef = useRef<(() => void) | null>(null);
  const releasePendingRef = useRef(false);
  const releaseTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const attemptRef = useRef(0);
  const invalidateCapture = () => {
    attemptRef.current++;
  };
  const ignoreActivationUntilRef = useRef(0);
  const commitRef = useRef<(key: string) => void>(() => {});

  const releaseCapture = () => {
    if (releaseTimerRef.current) clearTimeout(releaseTimerRef.current);
    unsubscribeRef.current?.();
    unsubscribeRef.current = null;
    const id = leaseRef.current;
    leaseRef.current = null;
    if (id !== null) void api.endShortcutCapture(id).catch(() => {});
  };
  const commit = (key: string) => {
    if (releasePendingRef.current || leaseRef.current === null) return;
    releasePendingRef.current = true;
    ignoreActivationUntilRef.current = Date.now() + 300;
    if (releaseTimerRef.current) clearTimeout(releaseTimerRef.current);
    setRecording(false);
    heldRef.current = [];
    // Keep the native listener paused until the captured keys are released.
    // Some desktops swallow key-up; the timer bounds that fallback.
    releaseTimerRef.current = setTimeout(releaseCapture, 2000);
    onChange(key);
  };
  useLayoutEffect(() => {
    commitRef.current = commit;
  });
  useEffect(() => {
    const cancel = () => {
      invalidateCapture();
      releaseCapture();
      releasePendingRef.current = false;
      setRecording(false);
      setPreparing(false);
    };
    const release = (event: KeyboardEvent) => {
      if (!releasePendingRef.current) return;
      event.preventDefault();
      event.stopPropagation();
      if (!event.ctrlKey && !event.altKey && !event.shiftKey && !event.metaKey) releaseCapture();
    };
    window.addEventListener("blur", cancel);
    window.addEventListener("keyup", release, true);
    return () => {
      invalidateCapture();
      releaseCapture();
      window.removeEventListener("blur", cancel);
      window.removeEventListener("keyup", release, true);
    };
  }, []);

  const startCapture = async () => {
    if (recording || preparing) return;
    releaseCapture();
    releasePendingRef.current = false;
    const attempt = ++attemptRef.current;
    setPreparing(true);
    setNeedsKey(false);
    setError(null);
    heldRef.current = [];
    let unsubscribe: (() => void) | undefined;
    let id: number | undefined;
    try {
      unsubscribe = await safeListen<{ id: number; shortcut: string; pressed: boolean }>(
        "hotkey:capture",
        (event) => {
          if (event.id !== leaseRef.current) return;
          if (event.pressed) {
            if (!allowEscape && /^esc(ape)?$/i.test(event.shortcut.trim())) {
              releaseCapture();
              setRecording(false);
            } else {
              commitRef.current(event.shortcut);
            }
          } else if (releasePendingRef.current) releaseCapture();
        },
      );
      if (attempt !== attemptRef.current) {
        unsubscribe();
        return;
      }
      id = await api.beginShortcutCapture();
      if (attempt !== attemptRef.current) {
        unsubscribe();
        void api.endShortcutCapture(id).catch(() => {});
        return;
      }
      leaseRef.current = id;
      unsubscribeRef.current = unsubscribe;
      setRecording(true);
      // A crashed or abandoned picker must never leave dictation paused.
      releaseTimerRef.current = setTimeout(() => {
        releaseCapture();
        setRecording(false);
      }, 90_000);
    } catch {
      unsubscribe?.();
      if (id !== undefined) void api.endShortcutCapture(id).catch(() => {});
      if (attempt === attemptRef.current)
        setError("Couldn’t listen for keys. Finish recording, then try again.");
    } finally {
      if (attempt === attemptRef.current) setPreparing(false);
    }
  };

  useLayoutEffect(() => {
    if (!recording || disabled) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (
        e.key === "Escape" &&
        !allowEscape &&
        !e.ctrlKey &&
        !e.altKey &&
        !e.shiftKey &&
        !e.metaKey
      ) {
        setRecording(false);
        releaseCapture();
        heldRef.current = [];
        return;
      }
      const modNames: string[] = [];
      if (e.ctrlKey) modNames.push("Ctrl");
      if (e.altKey) modNames.push("Alt");
      if (e.shiftKey) modNames.push("Shift");
      if (e.metaKey) modNames.push("Win");
      if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) {
        heldRef.current = modNames;
        return;
      }
      let key = e.key;
      if (key === " ") key = "Space";
      else if (key.length === 1) key = key.toUpperCase();
      else key = key.charAt(0).toUpperCase() + key.slice(1);
      const canon = ORDER.filter((m) => modNames.includes(m));
      commitRef.current([...canon, key].join("+"));
    };
    const onKeyUp = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      const mod = MODIFIER_KEYS[e.key];
      if (mod) {
        // Some desktop shortcuts consume a modifier key-down. Recover the
        // complete chord from the release event rather than losing that key.
        const released = new Set(heldRef.current);
        released.add(mod);
        if (e.ctrlKey) released.add("Ctrl");
        if (e.altKey) released.add("Alt");
        if (e.shiftKey) released.add("Shift");
        if (e.metaKey) released.add("Win");
        if (released.size >= 2) {
          if (!allowModifierOnly) {
            heldRef.current = [];
            setNeedsKey(true);
            return;
          }
          commitRef.current(ORDER.filter((m) => released.has(m)).join("+"));
        } else {
          heldRef.current = heldRef.current.filter((m) => m !== mod);
        }
      }
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onKeyUp, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onKeyUp, true);
    };
  }, [recording, onChange, allowModifierOnly, disabled, allowEscape]);

  const sizeClass = size === "sm" ? "!w-[140px] !text-sm" : "!w-[180px]";

  return (
    <span className="inline-flex flex-col items-center gap-2">
      <button
        type="button"
        disabled={disabled || preparing}
        aria-label={ariaLabel}
        onClick={(event) => {
          // Keyboard activation after Space/Enter capture is not a second click.
          if (event.detail === 0 && Date.now() < ignoreActivationUntilRef.current) return;
          void startCapture();
        }}
        title={
          recording
            ? needsKey
              ? "Include a letter, number or Space (Esc to cancel)"
              : "Press keys… (Esc to cancel)"
            : "Click to record a new shortcut"
        }
        className={`field ${sizeClass} !text-center font-semibold cursor-pointer transition-colors ${
          recording ? "!border-accent !text-accent ring-2 ring-accent/30" : ""
        }`}
      >
        <span className="inline-flex items-center gap-1.5">
          {recording && <Keyboard className="w-3.5 h-3.5" />}
          {preparing
            ? "Getting ready…"
            : recording
              ? needsKey
                ? "Include a key… (Esc)"
                : "Press keys… (Esc)"
              : value}
        </span>
      </button>
      {error && (
        <span role="alert" className="text-xs text-danger">
          {error}
        </span>
      )}
    </span>
  );
};
