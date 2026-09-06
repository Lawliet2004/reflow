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
  general: <Keyboard className="w-4 h-4" />,
  appearance: <Palette className="w-4 h-4" />,
  audio: <Mic className="w-4 h-4" />,
  model: <Cpu className="w-4 h-4" />,
  cleanup: <Sparkles className="w-4 h-4" />,
  dictionary: <BookOpen className="w-4 h-4" />,
  phone: <Smartphone className="w-4 h-4" />,
  advanced: <ShieldCheck className="w-4 h-4" />,
};

export const PAGES: { id: string; label: string; keywords: string[] }[] = [
  {
    id: "general",
    label: "General",
    keywords: ["hotkey", "shortcut", "window", "startup", "minimized"],
  },
  {
    id: "appearance",
    label: "Appearance",
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
    label: "Audio",
    keywords: ["mic", "microphone", "input", "gain", "silence", "language"],
  },
  { id: "model", label: "Model", keywords: ["asr", "qwen", "compute", "gpu", "cuda", "download"] },
  {
    id: "cleanup",
    label: "Cleanup",
    keywords: ["filler", "punctuation", "flow", "rewrite", "style"],
  },
  { id: "dictionary", label: "Dictionary", keywords: ["term", "vocab", "replace", "replacement"] },
  { id: "phone", label: "Phone", keywords: ["lan", "api", "pair", "qr", "device", "android"] },
  {
    id: "advanced",
    label: "Advanced",
    keywords: ["history", "retention", "clipboard", "developer", "logs", "diagnostics"],
  },
];

export function qrSrc(svg: string): string {
  if (svg.startsWith("data:")) return svg;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}
