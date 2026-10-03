import React, { useEffect, useState } from "react";
import { tauriApi } from "../../services/tauriApi";
import { Capabilities, Preset, RuntimePlan } from "../../types";
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
      "Prioritizes throughput using hardware, memory headroom and measured ASR performance.",
    icon: <Sparkles className="w-5 h-5 text-accent" />,
  },
  {
    id: "fast",
    shortcut: "2",
    title: "Fast",
    description:
      "Uses the smallest installed speech model and deterministic cleanup without the LLM wait.",
    icon: <Zap className="w-5 h-5 text-warning" />,
  },
  {
    id: "balanced",
    shortcut: "3",
    title: "Balanced",
    description: "Balances speech accuracy and memory use while keeping your chosen cleanup level.",
    icon: <ShieldCheck className="w-5 h-5 text-success" />,
  },
  {
    id: "accurate",
    shortcut: "4",
    title: "Accurate",
    description:
      "Prefers the larger installed speech model when memory allows. Keeps your chosen cleanup level.",
    icon: <Cpu className="w-5 h-5 text-indigo-500" />,
  },
  {
    id: "custom",
    shortcut: "5",
    title: "Custom",
    description:
      "Fine-grained manual control over models, compute devices, precision, and GPU layer offload.",
    icon: <Sliders className="w-5 h-5 text-muted" />,
  },
];

export function PresetSelector({
  selectedPreset,
  onSelectPreset,
  capabilities,
  className,
}: PresetSelectorProps) {
  const [profilePreview, setProfilePreview] = useState<RuntimePlan | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    let active = true;
    queueMicrotask(() => {
      if (active) setLoading(true);
    });
    const refresh = async () => {
      try {
        const res = await tauriApi.getRuntimePlan();
        if (active) setProfilePreview(res);
      } catch {
        if (active) setProfilePreview(null);
      } finally {
        if (active) setLoading(false);
      }
    };
    void refresh();
    const timer = setInterval(() => {
      void refresh();
    }, 5000);
    return () => {
      active = false;
      clearInterval(timer);
    };
  }, [selectedPreset, capabilities]);

  const handleKeyDown = (e: React.KeyboardEvent, index: number) => {
    const total = PRESET_OPTIONS.length;
    if (e.key === "ArrowRight" || e.key === "ArrowDown") {
      e.preventDefault();
      const nextIndex = (index + 1) % total;
      const radios = e.currentTarget.parentElement?.querySelectorAll<HTMLElement>('[role="radio"]');
      radios?.item(nextIndex).focus();
      onSelectPreset(PRESET_OPTIONS[nextIndex].id);
    } else if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
      e.preventDefault();
      const prevIndex = (index - 1 + total) % total;
      const radios = e.currentTarget.parentElement?.querySelectorAll<HTMLElement>('[role="radio"]');
      radios?.item(prevIndex).focus();
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
                "relative flex flex-col p-4 rounded-xl border transition-all cursor-pointer select-none text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 dark:focus-visible:ring-offset-slate-900",
                isSelected
                  ? "bg-accent-soft border-accent shadow-sm ring-1 ring-accent"
                  : "bg-surface border-line hover:border-line-strong",
              )}
            >
              <div className="flex items-center justify-between gap-2 mb-2">
                <div className="flex items-center gap-2 font-medium text-ink">
                  {opt.icon}
                  <span>{opt.title}</span>
                </div>
                <div className="flex items-center gap-1.5">
                  <kbd className="px-1.5 py-0.5 text-2xs font-mono rounded bg-base-2 text-muted border border-line">
                    {opt.shortcut}
                  </kbd>
                  {opt.badge ? (
                    <Badge variant="primary" size="sm">
                      {opt.badge}
                    </Badge>
                  ) : null}
                </div>
              </div>
              <p id={descId} className="text-xs text-ink-2 leading-relaxed flex-1">
                {opt.description}
              </p>
            </div>
          );
        })}
      </div>

      {profilePreview ? (
        <div
          aria-live="polite"
          className="p-4 rounded-xl bg-surface-2 border border-line text-xs space-y-2"
        >
          <div className="flex items-center justify-between font-medium text-ink-2">
            <span>Current hardware plan</span>
            {loading ? (
              <span className="text-muted animate-pulse">Resolving...</span>
            ) : (
              <Badge variant="outline">
                {profilePreview.asr_device.toUpperCase()} ASR ·{" "}
                {profilePreview.refinement_model
                  ? `${profilePreview.refinement_device.toUpperCase()} Polish`
                  : "ASR Only"}
              </Badge>
            )}
          </div>
          <div className="grid grid-cols-2 md:grid-cols-4 gap-2 pt-1 text-ink-2">
            <div>
              <span className="text-muted block">ASR Model:</span>
              <span className="font-medium text-ink">{profilePreview.asr_model}</span>
            </div>
            <div>
              <span className="text-muted block">Precision:</span>
              <span className="font-medium text-ink">
                {profilePreview.asr_precision.toUpperCase()}
              </span>
            </div>
            <div>
              <span className="text-muted block">Refinement:</span>
              <span className="font-medium text-ink">
                {profilePreview.refinement_model || "None (ASR-only)"}
              </span>
            </div>
            <div>
              <span className="text-muted block">Context / CPU threads:</span>
              <span className="font-medium text-ink">
                {profilePreview.context_size} tokens / {profilePreview.inference_threads}
              </span>
            </div>
          </div>
          {profilePreview.error && (
            <ul className="pt-2 space-y-1 border-t border-danger/30">
              <li className="text-danger font-medium">{profilePreview.error}</li>
            </ul>
          )}
          {profilePreview.reasons.length > 0 && (
            <ul className="pt-2 space-y-1 border-t border-line">
              {profilePreview.reasons.map((r) => (
                <li key={r.code} className="text-muted leading-snug">
                  {r.detail}
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}
    </div>
  );
}
