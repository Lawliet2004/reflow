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
    icon: <Sparkles className="w-4 h-4" />,
  },
  {
    id: "fast",
    shortcut: "2",
    title: "Fast",
    description:
      "Uses the smallest installed speech model and deterministic cleanup without the LLM wait.",
    icon: <Zap className="w-4 h-4" />,
  },
  {
    id: "balanced",
    shortcut: "3",
    title: "Balanced",
    description: "Balances speech accuracy and memory use while keeping your chosen cleanup level.",
    icon: <ShieldCheck className="w-4 h-4" />,
  },
  {
    id: "accurate",
    shortcut: "4",
    title: "Accurate",
    description:
      "Prefers the larger installed speech model when memory allows. Keeps your chosen cleanup level.",
    icon: <Cpu className="w-4 h-4" />,
  },
  {
    id: "custom",
    shortcut: "5",
    title: "Custom",
    description:
      "Fine-grained manual control over models, compute devices, precision, and GPU layer offload.",
    icon: <Sliders className="w-4 h-4" />,
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
        className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-2"
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
                "relative flex flex-col p-3 rounded-[var(--radius-control)] border transition-colors cursor-pointer select-none text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent",
                isSelected
                  ? "bg-accent-soft border-accent"
                  : "bg-surface border-line hover:border-line-strong",
              )}
            >
              <div className="flex items-center justify-between gap-2 mb-1.5">
                <div className="flex items-center gap-2 text-sm font-medium text-ink">
                  <span aria-hidden className={isSelected ? "text-accent" : "text-muted"}>
                    {opt.icon}
                  </span>
                  <span>{opt.title}</span>
                </div>
                {opt.badge ? (
                  <Badge variant="primary" size="sm">
                    {opt.badge}
                  </Badge>
                ) : null}
              </div>
              <p id={descId} className="text-xs text-muted leading-relaxed flex-1">
                {opt.description}
              </p>
            </div>
          );
        })}
      </div>

      {profilePreview ? (
        <div aria-live="polite" className="text-xs space-y-3">
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
          <dl className="grid grid-cols-2 md:grid-cols-4 gap-x-3 gap-y-2">
            <div>
              <dt className="text-muted">Speech model</dt>
              <dd className="font-medium text-ink tabular-nums">{profilePreview.asr_model}</dd>
            </div>
            <div>
              <dt className="text-muted">Precision</dt>
              <dd className="font-medium text-ink tabular-nums">
                {profilePreview.asr_precision.toUpperCase()}
              </dd>
            </div>
            <div>
              <dt className="text-muted">Writing model</dt>
              <dd className="font-medium text-ink tabular-nums">
                {profilePreview.refinement_model || "None (ASR-only)"}
              </dd>
            </div>
            <div>
              <dt className="text-muted">Context · threads</dt>
              <dd className="font-medium text-ink tabular-nums">
                {profilePreview.context_size} · {profilePreview.inference_threads}
              </dd>
            </div>
          </dl>
          {profilePreview.error && (
            <p className="text-danger font-medium">{profilePreview.error}</p>
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
