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
 * are held simultaneously.
 */
export const HotkeyPicker: React.FC<HotkeyPickerProps> = ({ value, onChange, size = "md" }) => {
  const [recording, setRecording] = useState(false);
  const [heldMods, setHeldMods] = useState<string[]>([]);
  const heldRef = useRef<string[]>([]);
  heldRef.current = heldMods;

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      e.preventDefault();
      e.stopPropagation();
      if (e.key === "Escape") {
        setRecording(false);
        setHeldMods([]);
        return;
      }
      const modNames: string[] = [];
      if (e.ctrlKey) modNames.push("Ctrl");
      if (e.altKey) modNames.push("Alt");
      if (e.shiftKey) modNames.push("Shift");
      if (e.metaKey) modNames.push("Win");
      if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) {
        const held = Array.from(new Set([...heldRef.current, ...modNames]));
        setHeldMods(held);
        if (held.length >= 2) {
          const combo = ORDER.filter((m) => held.includes(m)).join("+");
          onChange(combo);
          setRecording(false);
          setHeldMods([]);
        }
        return;
      }
      let key = e.key;
      if (key === " ") key = "Space";
      else if (key.length === 1) key = key.toUpperCase();
      else key = key.charAt(0).toUpperCase() + key.slice(1);
      const canon = ORDER.filter((m) => modNames.includes(m));
      onChange([...canon, key].join("+"));
      setRecording(false);
      setHeldMods([]);
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
        setHeldMods((prev) => prev.filter((m) => m !== mod));
      }
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("keyup", onKeyUp, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("keyup", onKeyUp, true);
    };
  }, [recording, onChange]);

  const sizeClass = size === "sm" ? "!w-[140px] !text-[12.5px]" : "!w-[180px]";

  return (
    <button
      type="button"
      onClick={() => {
        setRecording((v) => !v);
        setHeldMods([]);
      }}
      title={recording ? "Press keys… (Esc to cancel)" : "Click to record a new shortcut"}
      className={`field ${sizeClass} !text-center font-semibold cursor-pointer transition-all ${
        recording ? "!border-sky-500 !text-sky-600 ring-2 ring-sky-400/30" : ""
      }`}
    >
      <span className="inline-flex items-center gap-1.5">
        {recording && <Keyboard className="w-3.5 h-3.5" />}
        {recording ? "Press keys… (Esc)" : value}
      </span>
    </button>
  );
};
