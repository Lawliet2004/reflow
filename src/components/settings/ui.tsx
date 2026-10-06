import React from "react";
import {
  Keyboard,
  Mic,
  Cpu,
  ShieldCheck,
  BookOpen,
  Sparkles,
  Smartphone,
  Palette,
} from "lucide-react";
import { Section, Row, Switch } from "../ui";

export { Section, Row };
export const Toggle = Switch;

const ICON = { size: 17, strokeWidth: 1.75, "aria-hidden": true } as const;
export const PAGE_ICONS: Record<string, React.ReactNode> = {
  general: <Keyboard {...ICON} />,
  appearance: <Palette {...ICON} />,
  audio: <Mic {...ICON} />,
  model: <Cpu {...ICON} />,
  cleanup: <Sparkles {...ICON} />,
  dictionary: <BookOpen {...ICON} />,
  phone: <Smartphone {...ICON} />,
  advanced: <ShieldCheck {...ICON} />,
};

export const PAGES: { id: string; label: string; description: string; keywords: string[] }[] = [
  {
    id: "general",
    label: "General",
    description: "Hotkeys, dictation output, modes, snippets and startup.",
    keywords: [
      "hotkey",
      "shortcut",
      "window",
      "startup",
      "minimized",
      "output",
      "paste",
      "send",
      "copy",
      "file",
      "mode",
      "context",
      "command",
      "trigger",
      "instructions",
      "snippet",
      "expansion",
      "address",
      "signature",
    ],
  },
  {
    id: "appearance",
    label: "Appearance",
    description: "Theme, paper, ink, type, spacing and the dictation pill.",
    keywords: [
      "theme",
      "dark",
      "light",
      "color",
      "colour",
      "accent",
      "paper",
      "tone",
      "mica",
      "hud",
      "pill",
      "waveform",
      "shape",
      "overlay",
      "position",
      "opacity",
      "contrast",
      "font",
      "serif",
      "scale",
      "size",
      "density",
      "corner",
      "radius",
      "motion",
      "reset",
    ],
  },
  {
    id: "audio",
    label: "Speech",
    description: "Microphone, input level, silence handling and dictation language.",
    keywords: ["audio", "mic", "microphone", "input", "gain", "silence", "language"],
  },
  {
    id: "model",
    label: "Performance",
    description: "Speech model, compute device, downloads and calibration.",
    keywords: [
      "model",
      "asr",
      "qwen",
      "compute",
      "gpu",
      "cuda",
      "download",
      "runtime",
      "calibration",
    ],
  },
  {
    id: "cleanup",
    label: "Writing",
    description: "How much Reflow tidies and rewrites what you say.",
    keywords: ["cleanup", "filler", "punctuation", "flow", "rewrite", "style"],
  },
  {
    id: "dictionary",
    label: "Dictionary",
    description: "Names and terms Reflow should always spell your way.",
    keywords: ["term", "vocab", "replace", "replacement"],
  },
  {
    id: "phone",
    label: "Phone",
    description: "Pair a phone to dictate into this computer over your network.",
    keywords: ["lan", "api", "pair", "qr", "device", "android"],
  },
  {
    id: "advanced",
    label: "Advanced",
    description: "History retention, privacy, clipboard and diagnostics.",
    keywords: [
      "history",
      "retention",
      "audio",
      "recordings",
      "privacy",
      "clipboard",
      "developer",
      "logs",
      "diagnostics",
    ],
  },
];

export const PAGE_GROUPS: { label: string; ids: string[] }[] = [
  { label: "Dictation", ids: ["general", "audio", "cleanup", "dictionary"] },
  { label: "App", ids: ["appearance", "model", "phone", "advanced"] },
];

export function qrSrc(svg: string): string {
  if (svg.startsWith("data:")) return svg;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}
