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

export const PAGE_ICONS: Record<string, React.ReactNode> = {
  modes: <Sparkles className="w-4 h-4" />,
  snippets: <BookOpen className="w-4 h-4" />,
  output: <Keyboard className="w-4 h-4" />,
  general: <Keyboard className="w-4 h-4" />,
  appearance: <Palette className="w-4 h-4" />,
  audio: <Mic className="w-4 h-4" />,
  model: <Cpu className="w-4 h-4" />,
  cleanup: <Sparkles className="w-4 h-4" />,
  dictionary: <BookOpen className="w-4 h-4" />,
  phone: <Smartphone className="w-4 h-4" />,
  advanced: <ShieldCheck className="w-4 h-4" />,
};

export const PAGES: { id: string; label: string; description: string; keywords: string[] }[] = [
  {
    id: "modes",
    label: "Modes",
    description: "Custom instructions, context, app triggers and mode shortcuts.",
    keywords: ["mode", "context", "command", "trigger", "instructions"],
  },
  {
    id: "snippets",
    label: "Snippets",
    description: "Spoken phrases that insert stored text.",
    keywords: ["snippet", "expansion", "address", "signature"],
  },
  {
    id: "output",
    label: "Output",
    description: "Paste, send, copy, HUD, files and command destinations.",
    keywords: ["output", "paste", "send", "copy", "command", "file"],
  },
  {
    id: "general",
    label: "General",
    description: "Your hotkey, and how Reflow starts and sits on your desktop.",
    keywords: ["hotkey", "shortcut", "window", "startup", "minimized"],
  },
  {
    id: "appearance",
    label: "Appearance",
    description: "Theme, accent colour, text size, motion and the recording HUD.",
    keywords: [
      "theme",
      "dark",
      "light",
      "color",
      "accent",
      "hud",
      "overlay",
      "font",
      "scale",
      "motion",
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

export function qrSrc(svg: string): string {
  if (svg.startsWith("data:")) return svg;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}
