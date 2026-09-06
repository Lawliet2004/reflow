import React, { useEffect, useState } from "react";
import { tauriApi } from "../../services/tauriApi";
import { Capabilities, Preset, ResolvedProfile } from "../../types";
import { Badge } from "../ui/Badge";
import { cn } from "../../utils/cn";
import { Zap, Cpu, Sparkles, Sliders, ShieldCheck } from "lucide-react";

interface PresetSelectorProps {
  selectedPreset: Preset;
  onSelectPreset: (preset: Preset) => void;
  capabilities?: Capabilities | null;
  className?: string;
}

interface PresetOption {
  id: Preset;
  shortcut: string;
  title: string;
  badge?: string;
  description: string;
  icon: React.ReactNode;
}

const PRESET_OPTIONS: PresetOption[] = [
  {
    id: "auto",
    shortcut: "1",
    title: "Auto (Adaptive)",
    badge: "Recommended",
    description:
      "Dynamically selects optimal models based on live VRAM, RAM, and hardware capabilities.",
    icon: <Sparkles className="w-5 h-5 text-sky-500" />,
  },
  {
    id: "fast",
    shortcut: "2",
    title: "Fast",
    description:
      "Ultra-low latency ASR-only mode (~200ms). Runs lightweight 0.6B on CPU or GPU without LLM delay.",
    icon: <Zap className="w-5 h-5 text-amber-500" />,
  },
  {
    id: "balanced",
    shortcut: "3",
    title: "Balanced",
    description:
      "Fast ASR paired with efficient Qwen3.5-0.8B refinement for natural punctuation and phrasing.",
    icon: <ShieldCheck className="w-5 h-5 text-emerald-500" />,
  },
  {
    id: "accurate",
    shortcut: "4",
    title: "Accurate",
    description:
      "Highest quality transcription with full Qwen3.5-2B LLM polish. Best for complex dictation.",
    icon: <Cpu className="w-5 h-5 text-indigo-500" />,
  },
  {
    id: "custom",
    shortcut: "5",
    title: "Custom",
    description:
      "Fine-grained manual control over models, compute devices, precision, and GPU layer offload.",
    icon: <Sliders className="w-5 h-5 text-slate-500" />,
  },
];

export function PresetSelector({
  selectedPreset,
  onSelectPreset,
  capabilities,
  className,
}: PresetSelectorProps) {
  const [profilePreview, setProfilePreview] = useState<ResolvedProfile | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    let active = true;
    setLoading(true);
    tauriApi.previewProfile(selectedPreset).then((res: ResolvedProfile | null) => {
      if (active) {
        setProfilePreview(res);
        setLoading(false);
      }
    });
    return () => {
      active = false;
    };
  }, [selectedPreset, capabilities]);

  const handleKeyDown = (e: React.KeyboardEvent, index: number) => {
    const total = PRESET_OPTIONS.length;
    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      const nextIndex = (index + 1) % total;
      onSelectPreset(PRESET_OPTIONS[nextIndex].id);
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      const prevIndex = (index - 1 + total) % total;
      onSelectPreset(PRESET_OPTIONS[prevIndex].id);
    } else if (e.key === " " || e.key === "Enter") {
      e.preventDefault();
      onSelectPreset(PRESET_OPTIONS[index].id);
    } else {
      const match = PRESET_OPTIONS.find((opt) => opt.shortcut === e.key);
      if (match) {
        e.preventDefault();
        onSelectPreset(match.id);
      }
    }
  };

  return (
    <div className={cn("space-y-4", className)}>
      <div
        role="radiogroup"
        aria-label="Hardware intelligence presets"
        className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-3"
      >
        {PRESET_OPTIONS.map((opt, index) => {
          const isSelected = selectedPreset === opt.id;
          const descId = `preset-desc-${opt.id}`;
          return (
            <div
              key={opt.id}
              role="radio"
              aria-checked={isSelected}
              aria-describedby={descId}
              tabIndex={isSelected ? 0 : -1}
              onClick={() => onSelectPreset(opt.id)}
              onKeyDown={(e) => handleKeyDown(e, index)}
              className={cn(
                "relative flex flex-col p-4 rounded-xl border transition-all cursor-pointer select-none text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sky-500 focus-visible:ring-offset-2 dark:focus-visible:ring-offset-slate-900",
                isSelected
                  ? "bg-sky-50/50 dark:bg-sky-950/20 border-sky-500 dark:border-sky-500 shadow-sm ring-1 ring-sky-500"
                  : "bg-white dark:bg-slate-900 border-slate-200 dark:border-slate-800 hover:border-slate-300 dark:hover:border-slate-700",
              )}
            >
              <div className="flex items-center justify-between gap-2 mb-2">
                <div className="flex items-center gap-2 font-medium text-slate-900 dark:text-slate-100">
                  {opt.icon}
                  <span>{opt.title}</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <kbd className="px-1.5 py-0.5 text-[10px] font-mono rounded bg-slate-100 dark:bg-slate-800 text-slate-500 border border-slate-200 dark:border-slate-700">
                    {opt.shortcut}
                  </kbd>
                  {opt.badge ? (
                    <Badge variant="primary" size="sm">
                      {opt.badge}
                    </Badge>
                  ) : null}
                </div>
              </div>
              <p
                id={descId}
                className="text-xs text-slate-600 dark:text-slate-400 leading-relaxed flex-1"
              >
                {opt.description}
              </p>
            </div>
          );
        })}
      </div>

      {profilePreview ? (
        <div
          aria-live="polite"
          className="p-4 rounded-xl bg-slate-50 dark:bg-slate-900/60 border border-slate-200 dark:border-slate-800 text-xs space-y-2"
        >
          <div className="flex items-center justify-between font-medium text-slate-700 dark:text-slate-300">
            <span>Resolved Hardware Plan:</span>
            {loading ? (
              <span className="text-slate-400 animate-pulse">Resolving...</span>
            ) : (
              <Badge variant="outline">
                {profilePreview.asr_device.toUpperCase()} ASR ?{" "}
                {profilePreview.refinement_model
                  ? `${profilePreview.refinement_device.toUpperCase()} Polish`
                  : "ASR Only"}
              </Badge>
            )}
          </div>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-2 pt-1 text-slate-600 dark:text-slate-400">
            <div>
              <span className="text-slate-400 block">ASR Model:</span>
              <span className="font-medium text-slate-800 dark:text-slate-200">
                {profilePreview.asr_model}
              </span>
            </div>
            <div>
              <span className="text-slate-400 block">Precision:</span>
              <span className="font-medium text-slate-800 dark:text-slate-200">
                {profilePreview.asr_precision.toUpperCase()}
              </span>
            </div>
            <div>
              <span className="text-slate-400 block">Refinement:</span>
              <span className="font-medium text-slate-800 dark:text-slate-200">
                {profilePreview.refinement_model || "None (ASR-only)"}
              </span>
            </div>
            <div>
              <span className="text-slate-400 block">Streaming:</span>
              <span className="font-medium text-slate-800 dark:text-slate-200">
                {profilePreview.streaming_enabled ? "Enabled" : "Off (Batch)"}
              </span>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
