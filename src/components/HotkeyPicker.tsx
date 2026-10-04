import React, { useEffect, useRef, useState } from "react";
import { Keyboard } from "lucide-react";

interface HotkeyPickerProps {
  value: string;
  onChange: (value: string) => void;
  size?: "sm" | "md";
}

const ORDER = ["Ctrl", "Alt", "Shift", "Win"];

/**
 * Click to arm, then press a key/combo. Esc cancels.
 * Modifier-only combos (e.g. Shift+Win) are detected when ≥2 modifiers
 * are held simultaneously and then released. Waiting for release also allows
 * shortcuts such as Ctrl+Shift+K to include a non-modifier key.
 */
export const HotkeyPicker: React.FC<HotkeyPickerProps> = ({ value, onChange, size = "md" }) => {
  const [recording, setRecording] = useState(false);
  const heldRef = useRef<string[]>([]);

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setRecording(false);
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
      onChange([...canon, key].join("+"));
      setRecording(false);
      heldRef.current = [];
    };
    const onKeyUp = (e: KeyboardEvent) => {
      const map: Record<string, string> = {
        Control: "Ctrl",
        Alt: "Alt",
        Shift: "Shift",
        Meta: "Win",
      };
      const mod = map[e.key];
      if (mod) {
        if (heldRef.current.length >= 2) {
          onChange(ORDER.filter((m) => heldRef.current.includes(m)).join("+"));
          setRecording(false);
          heldRef.current = [];
        } else {
          heldRef.current = heldRef.current.filter((m) => m !== mod);
        }
      }
    };
    const cancel = () => {
      setRecording(false);
      heldRef.current = [];
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onKeyUp, true);
    window.addEventListener("blur", cancel);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onKeyUp, true);
      window.removeEventListener("blur", cancel);
    };
  }, [recording, onChange]);

  const sizeClass = size === "sm" ? "!w-[140px] !text-sm" : "!w-[180px]";

  return (
    <button
      type="button"
      onClick={() => {
        setRecording((v) => !v);
        heldRef.current = [];
      }}
      title={recording ? "Press keys… (Esc to cancel)" : "Click to record a new shortcut"}
      className={`field ${sizeClass} !text-center font-semibold cursor-pointer transition-colors ${
        recording ? "!border-accent !text-accent ring-2 ring-accent/30" : ""
      }`}
    >
      <span className="inline-flex items-center gap-1.5">
        {recording && <Keyboard className="w-3.5 h-3.5" />}
        {recording ? "Press keys… (Esc)" : value}
      </span>
    </button>
  );
};
