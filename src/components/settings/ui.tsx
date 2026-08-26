import React from "react";
import {
  Keyboard,
  Mic,
  Cpu,
  ShieldCheck,
  BookOpen,
  Sliders,
  Sparkles,
  Smartphone,
  Palette,
} from "lucide-react";

export const Section: React.FC<{
  icon: React.ReactNode;
  title: string;
  description?: string;
  children: React.ReactNode;
}> = ({ icon, title, description, children }) => (
  <section className="panel p-5 shadow-xs">
    <div className="flex items-center gap-2.5 mb-4 pb-3 border-b border-line">
      <div className="w-7 h-7 rounded-lg bg-accent-soft border border-accent-border flex items-center justify-center text-accent">
        {icon}
      </div>
      <h2 className="text-[14.5px] font-semibold text-ink tracking-tight">{title}</h2>
    </div>
    {description && (
      <p className="text-[12.5px] text-muted leading-relaxed -mt-2 mb-4">{description}</p>
    )}
    <div className="space-y-4">{children}</div>
  </section>
);

export const Row: React.FC<{
  label: string;
  hint?: string;
  children: React.ReactNode;
}> = ({ label, hint, children }) => (
  <div className="flex items-center justify-between gap-6 py-0.5">
    <div className="min-w-0">
      <p className="text-[13px] text-ink font-medium">{label}</p>
      {hint && <p className="text-[11.5px] text-muted mt-0.5 leading-relaxed">{hint}</p>}
    </div>
    <div className="shrink-0">{children}</div>
  </div>
);

export const Toggle: React.FC<{ on: boolean; onChange: (v: boolean) => void; ariaLabel?: string }> = ({
  on,
  onChange,
  ariaLabel,
}) => (
  <button
    type="button"
    role="switch"
    aria-checked={on}
    aria-label={ariaLabel}
    onClick={() => onChange(!on)}
    className={`relative w-[40px] h-[22px] rounded-full transition-colors cursor-pointer ${
      on ? "bg-accent shadow-[0_2px_8px_rgba(2,132,199,0.35)]" : "bg-slate-300 dark:bg-slate-700"
    }`}
  >
    <span
      className={`absolute top-[2px] w-[18px] h-[18px] rounded-full bg-white shadow-xs transition-all ${
        on ? "left-[20px]" : "left-[2px]"
      }`}
    />
  </button>
);

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
  { id: "general", label: "General", keywords: ["hotkey", "shortcut", "window", "startup", "minimized"] },
  { id: "appearance", label: "Appearance", keywords: ["theme", "dark", "light", "color", "accent", "hud", "overlay", "font", "scale", "motion"] },
  { id: "audio", label: "Audio", keywords: ["mic", "microphone", "input", "gain", "silence", "language"] },
  { id: "model", label: "Model", keywords: ["asr", "qwen", "compute", "gpu", "cuda", "download"] },
  { id: "cleanup", label: "Cleanup", keywords: ["filler", "punctuation", "flow", "rewrite", "style"] },
  { id: "dictionary", label: "Dictionary", keywords: ["term", "vocab", "replace", "replacement"] },
  { id: "phone", label: "Phone", keywords: ["lan", "api", "pair", "qr", "device", "android"] },
  { id: "advanced", label: "Advanced", keywords: ["history", "retention", "clipboard", "developer", "logs", "diagnostics"] },
];

export function qrSrc(svg: string): string {
  if (svg.startsWith("data:")) return svg;
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}
