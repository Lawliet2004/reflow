import React, { useState } from "react";
import {
  AppSettings,
  IntelligenceTier,
  IntelligenceTierState,
  INTELLIGENCE_TIERS,
  ModelStatus,
  RuntimeDownloadEvent,
  TranscriptStyle,
} from "../../types";
import { api } from "../../services/tauriApi";
import { ApplicationProfiles } from "./ApplicationProfiles";
import { Section, Row, Toggle } from "./ui";
import {
  Cpu,
  Download,
  Globe,
  Loader2,
  Mic,
  Play,
  ShieldCheck,
  Sparkles,
  Trash2,
  Check,
  AlertTriangle,
} from "lucide-react";
import type { IntelligenceDownloadEvent } from "../../App";
import type { IntelligenceHub } from "../../hooks/useIntelligenceHub";

interface Props {
  intelligence: IntelligenceHub;
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => Promise<boolean>;
  modelStatus: ModelStatus | null;
  intelligenceDownload: IntelligenceDownloadEvent | null;
  activeDownloadTiers: Set<IntelligenceTier>;
  runtimeDownload: RuntimeDownloadEvent | null;
  runtimeDownloadActive: boolean;
  runtimeDownloadError: string | null;
  onInstallRuntime: () => void;
  onRemoveRuntime: () => void;
}

const STYLE_OPTIONS: { value: TranscriptStyle; label: string }[] = [
  { value: "faithful", label: "Faithful" },
  { value: "neutral", label: "Neutral" },
  { value: "decisive", label: "Decisive" },
  { value: "email", label: "Email" },
  { value: "chat", label: "Chat" },
];

const TIER_ICONS: Record<IntelligenceTier, React.ReactNode> = {
  raw_verbatim: <Mic className="w-4 h-4" />,
  smart_flow: <Sparkles className="w-4 h-4" />,
  deep_context: <Globe className="w-4 h-4" />,
};

const formatSize = (mb: number) => (mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${mb} MB`);

export const CleanupPage: React.FC<Props> = ({
  settings,
  intelligence,
  onUpdateSettings,
  modelStatus,
  intelligenceDownload,
  activeDownloadTiers,
  runtimeDownload,
  runtimeDownloadActive,
  runtimeDownloadError,
  onInstallRuntime,
  onRemoveRuntime: _onRemoveRuntime,
}) => {
  const { flowStatus, intelligenceTiers, systemMetrics } = intelligence;

  const [installing, setInstalling] = useState<IntelligenceTier | null>(null);
  const [sample, setSample] = useState("I want to drink coffee um no wait I want tea");
  const [previewOut, setPreviewOut] = useState("");
  const [previewLatency, setPreviewLatency] = useState<number | null>(null);
  const [previewModel, setPreviewModel] = useState<string>("");
  const [previewing, setPreviewing] = useState(false);
  const [removing, setRemoving] = useState<IntelligenceTier | null>(null);

  const tier: IntelligenceTier = settings.intelligence_tier ?? "smart_flow";

  const hasGpu = Boolean(systemMetrics?.gpu_present || modelStatus?.gpu_available);
  const totalRamMb = systemMetrics?.total_ram_mb ?? 0;
  const lowSpecPc = !hasGpu || totalRamMb < 8 * 1024;

  const tierState = (t: IntelligenceTier): IntelligenceTierState | null => {
    if (t === "raw_verbatim") {
      return { tier: t, model_id: "none", installed: true, downloading: false };
    }
    return intelligenceTiers?.find((x) => x.tier === t) ?? null;
  };

  const isInstalled = (t: IntelligenceTier) => tierState(t)?.installed ?? false;

  // The runtime is a single binary that gates every non-raw tier. We
  // know a tier is "weights only" when its GGUF is on disk (per the
  // per-tier state, which the Rust side keeps truthful) and the
  // runtime is not.
  const runtimeInstalled = flowStatus?.runtime_installed ?? false;
  const isWeightsOnly = (t: IntelligenceTier) => {
    if (t === "raw_verbatim") return false;
    const ggufInstalled = tierState(t)?.weights_installed ?? tierState(t)?.installed ?? false;
    if (!ggufInstalled) return false;
    return !runtimeInstalled;
  };

  const isDownloading = (t: IntelligenceTier) => activeDownloadTiers.has(t);

  const downloadProgress = (t: IntelligenceTier) =>
    isDownloading(t) && intelligenceDownload?.tier === t ? intelligenceDownload.progress_pct : 0;

  const downloadSpeed = (t: IntelligenceTier) =>
    isDownloading(t) && intelligenceDownload?.tier === t ? intelligenceDownload.speed_mbps : 0;

  const handleSelectTier = (t: IntelligenceTier) =>
    onUpdateSettings({
      intelligence_tier: t,
      cleanup_level:
        t === "raw_verbatim"
          ? "raw"
          : settings.cleanup_level === "raw"
            ? "medium"
            : settings.cleanup_level,
      ...(t !== "raw_verbatim" && settings.preset === "fast" ? { preset: "auto" as const } : {}),
    });

  const handleInstall = async (t: IntelligenceTier) => {
    if (t === "raw_verbatim") return;
    if (activeDownloadTiers.has(t)) return;
    if (tierState(t)?.installed) return;
    setInstalling(t);
    try {
      await api.installIntelligenceModel(t);
    } catch (e) {
      intelligence.notifyToast(
        "error",
        "The model operation could not be completed. Please try again or check the logs.",
      );
      console.error("installIntelligenceModel failed", e);
    } finally {
      setInstalling(null);
    }
  };

  const handleRemove = async (t: IntelligenceTier) => {
    if (t === "raw_verbatim") return;
    setRemoving(t);
    try {
      await api.removeIntelligenceModel(t);
      await intelligence.refresh();
    } catch (e) {
      intelligence.notifyToast(
        "error",
        "The model operation could not be completed. Please try again or check the logs.",
      );
      console.error("removeIntelligenceModel failed", e);
    } finally {
      setRemoving(null);
    }
  };

  const runPreview = async () => {
    setPreviewing(true);
    try {
      const result = await api.previewTierCleanup(sample, tier, settings.style);
      setPreviewOut(result.text);
      setPreviewLatency(result.latency_ms);
      setPreviewModel(result.model_used);
    } catch (e) {
      intelligence.notifyToast(
        "error",
        "The model operation could not be completed. Please try again or check the logs.",
      );
      console.error("preview failed", e);
      setPreviewOut("");
      setPreviewLatency(null);
      setPreviewModel("");
    } finally {
      setPreviewing(false);
    }
  };

  const activeMeta = INTELLIGENCE_TIERS[tier];

  return (
    <Section
      icon={<Sparkles className="w-4 h-4 text-accent" />}
      title="Intelligence & Cleanup Engine"
      description="Choose how Reflow polishes your dictation. Stage 1 rules always run. Stage 2 (optional) uses a small on-device LLM."
    >
      {runtimeDownloadError && (
        <p role="alert" className="text-sm text-danger">
          Runtime installation failed: {runtimeDownloadError}
        </p>
      )}
      <div className="grid grid-cols-1 gap-3">
        {(Object.keys(INTELLIGENCE_TIERS) as IntelligenceTier[]).map((id) => {
          const meta = INTELLIGENCE_TIERS[id];
          const active = tier === id;
          const installed = isInstalled(id);
          const downloading = isDownloading(id);
          const progress = downloadProgress(id);
          const speed = downloadSpeed(id);
          const isRemoving = removing === id;
          const isDeepContext = id === "deep_context";
          const needsGpu = isDeepContext && !hasGpu;
          return (
            <div
              key={id}
              className={`relative rounded-2xl border transition-colors p-4 ${
                active
                  ? "border-accent bg-accent-soft shadow-xs ring-1 ring-accent"
                  : "border-line bg-surface hover:border-line-strong hover:bg-base-2"
              }`}
            >
              <div className="flex items-start gap-3">
                <button
                  type="button"
                  onClick={() => handleSelectTier(id)}
                  aria-pressed={active}
                  className="flex items-start gap-3 text-left flex-1 min-w-0"
                >
                  <div
                    className={`w-9 h-9 rounded-xl flex items-center justify-center shrink-0 ${
                      active ? "bg-accent text-white" : "bg-surface-2 text-ink-2"
                    }`}
                  >
                    {TIER_ICONS[id]}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      <p
                        className={`text-base font-semibold ${active ? "text-accent" : "text-ink"}`}
                      >
                        {meta.label}
                      </p>
                      <span
                        className={`px-1.5 py-0.5 rounded text-2xs font-bold tracking-wider ${
                          id === "smart_flow"
                            ? "bg-success/15 text-success"
                            : id === "deep_context"
                              ? "bg-violet-500/15 text-violet-600 dark:text-violet-300"
                              : "bg-muted/15 text-ink-2"
                        }`}
                      >
                        {meta.badgeText}
                      </span>
                    </div>
                    <p className="text-xs text-muted mt-0.5 leading-snug">{meta.tagline}</p>
                    <p className="text-sm text-ink-2 mt-2 leading-relaxed">{meta.description}</p>
                    <div className="flex flex-wrap gap-1.5 mt-2.5">
                      <span className="px-2 py-0.5 rounded-full bg-base-2 border border-line text-2xs text-ink-2 font-medium">
                        {meta.latencyEstimate}
                      </span>
                      {meta.downloadSizeMB > 0 && (
                        <span className="px-2 py-0.5 rounded-full bg-base-2 border border-line text-2xs text-ink-2 font-medium">
                          {formatSize(meta.downloadSizeMB)}
                        </span>
                      )}
                      {meta.ramRequiredMB > 0 && (
                        <span className="px-2 py-0.5 rounded-full bg-base-2 border border-line text-2xs text-ink-2 font-medium">
                          {formatSize(meta.ramRequiredMB)} RAM
                        </span>
                      )}
                      {meta.vramRequiredMB > 0 && (
                        <span className="px-2 py-0.5 rounded-full bg-base-2 border border-line text-2xs text-ink-2 font-medium">
                          {formatSize(meta.vramRequiredMB)} VRAM
                        </span>
                      )}
                      {id === "raw_verbatim" && (
                        <span className="px-2 py-0.5 rounded-full bg-base-2 border border-line text-2xs text-ink-2 font-medium">
                          0 MB extra
                        </span>
                      )}
                      {id === "smart_flow" && (
                        <span className="px-2 py-0.5 rounded-full bg-base-2 border border-line text-2xs text-ink-2 font-medium">
                          86% IFEval
                        </span>
                      )}
                      {id === "deep_context" && (
                        <span className="px-2 py-0.5 rounded-full bg-base-2 border border-line text-2xs text-ink-2 font-medium">
                          201 Languages
                        </span>
                      )}
                    </div>
                    {needsGpu && (
                      <div className="mt-2.5 flex items-start gap-1.5 text-xs text-warning">
                        <AlertTriangle className="w-3.5 h-3.5 mt-0.5 shrink-0" />
                        <span>
                          {lowSpecPc
                            ? "Requires 4 GB+ free memory or a dedicated GPU. Slower on CPU-only laptops."
                            : "Best with a dedicated GPU. CPU inference will be slower."}
                        </span>
                      </div>
                    )}
                    <p className="text-2xs text-muted mt-2 italic">
                      Powered by {id === "raw_verbatim" ? "Stage 1 rules only" : meta.modelFile}
                    </p>
                  </div>
                </button>
                {active && (
                  <div className="w-5 h-5 rounded-full bg-accent text-white flex items-center justify-center shrink-0">
                    <Check className="w-3 h-3 stroke-[3]" />
                  </div>
                )}
              </div>

              {id !== "raw_verbatim" && (
                <div className="mt-3 pt-3 border-t border-line/60 flex items-center gap-2 flex-wrap">
                  {installed ? (
                    <>
                      <span className="inline-flex items-center gap-1.5 text-xs text-success font-medium">
                        <ShieldCheck className="w-3.5 h-3.5" />
                        Installed
                      </span>
                      <div className="flex-1" />
                      <button
                        type="button"
                        className="btn btn-ghost !py-1.5 !px-3 !text-xs"
                        onClick={() => handleRemove(id)}
                        disabled={isRemoving}
                      >
                        {isRemoving ? (
                          <Loader2 className="w-3.5 h-3.5 animate-spin" />
                        ) : (
                          <Trash2 className="w-3.5 h-3.5" />
                        )}
                        Remove
                      </button>
                    </>
                  ) : downloading ? (
                    <div className="w-full space-y-1.5">
                      <div className="flex items-center justify-between text-xs">
                        <span className="inline-flex items-center gap-1.5 text-accent font-medium">
                          <Loader2 className="w-3.5 h-3.5 animate-spin" />
                          Downloading {meta.modelFile}
                        </span>
                        <span className="text-muted">
                          {progress}% · {speed.toFixed(1)} MB/s
                        </span>
                      </div>
                      <div className="h-1.5 rounded-full bg-line overflow-hidden">
                        <div
                          className="progress-fill bg-accent"
                          style={{
                            transform: `scaleX(${Math.max(0, Math.min(100, progress)) / 100})`,
                          }}
                        />
                      </div>
                    </div>
                  ) : isWeightsOnly(id) ? (
                    <>
                      <span className="inline-flex items-center gap-1.5 text-xs text-warning font-medium">
                        <AlertTriangle className="w-3.5 h-3.5" />
                        Weights only — runtime missing
                      </span>
                      <div className="flex-1" />
                      {runtimeDownloadActive ? (
                        <span className="inline-flex items-center gap-1.5 text-2xs text-accent font-medium">
                          <Loader2 className="w-3 h-3 animate-spin" />
                          Installing runtime… {runtimeDownload?.progress_pct ?? 0}%
                        </span>
                      ) : (
                        <button
                          type="button"
                          className="btn btn-primary !py-1.5 !px-3 !text-xs"
                          onClick={onInstallRuntime}
                        >
                          <Cpu className="w-3.5 h-3.5" />
                          Install runtime
                        </button>
                      )}
                    </>
                  ) : (
                    <>
                      <span className="text-xs text-muted">
                        Download the GGUF weights to use this tier.
                      </span>
                      <div className="flex-1" />
                      <button
                        type="button"
                        className="btn btn-primary !py-1.5 !px-3 !text-xs"
                        onClick={() => handleInstall(id)}
                        disabled={installing === id}
                      >
                        {installing === id ? (
                          <Loader2 className="w-3.5 h-3.5 animate-spin" />
                        ) : (
                          <Download className="w-3.5 h-3.5" />
                        )}
                        Download ({formatSize(meta.downloadSizeMB)})
                      </button>
                    </>
                  )}
                </div>
              )}
            </div>
          );
        })}
      </div>

      {(tier === "smart_flow" || tier === "deep_context") && (
        <div className="mt-5 space-y-3">
          <div className="rounded-xl border border-line bg-surface overflow-hidden">
            <details className="group" open>
              <summary className="cursor-pointer px-3.5 py-2.5 flex items-center justify-between text-sm font-semibold text-ink select-none">
                <span>Voice tone &amp; style</span>
                <span className="text-2xs text-muted font-normal group-open:hidden">
                  Show options
                </span>
                <span className="text-2xs text-muted font-normal hidden group-open:inline">
                  Hide
                </span>
              </summary>
              <div className="px-3.5 pb-3.5 space-y-3 border-t border-line/60">
                <Row label="Style" hint="Adjusts the tone of the rewrite">
                  <select
                    className="field"
                    value={settings.style ?? "neutral"}
                    onChange={(e) => onUpdateSettings({ style: e.target.value as TranscriptStyle })}
                  >
                    {STYLE_OPTIONS.map((opt) => (
                      <option key={opt.value} value={opt.value}>
                        {opt.label}
                      </option>
                    ))}
                  </select>
                </Row>
                <Row
                  label="App-aware context"
                  hint="Adapt formatting to the focused window (chat, email, code)"
                >
                  <Toggle
                    on={settings.auto_style_from_app ?? true}
                    onChange={(v) => onUpdateSettings({ auto_style_from_app: v })}
                    ariaLabel="App-aware context"
                  />
                </Row>
              </div>
            </details>
          </div>
        </div>
      )}

      <div className="rounded-xl border border-line bg-surface p-3.5 space-y-2.5">
        <div className="flex items-center justify-between gap-2">
          <p className="text-sm font-semibold text-ink">Live playground</p>
          <span className="text-2xs text-muted">
            Tests {activeMeta.label}
            {flowStatus?.ready && flowStatus.backend
              ? ` · ${flowStatus.backend} ready`
              : tier === "raw_verbatim"
                ? " · Stage 1 rules"
                : " · model not loaded"}
          </span>
        </div>
        <textarea
          className="field w-full min-h-[64px] resize-y"
          value={sample}
          onChange={(e) => setSample(e.target.value)}
          placeholder="Paste a messy transcript and see how each tier cleans it up…"
        />
        <div className="flex items-center gap-2 flex-wrap">
          <button
            type="button"
            className="btn btn-primary !py-1.5 !px-3 !text-sm"
            onClick={runPreview}
            disabled={previewing || !sample.trim()}
          >
            {previewing ? (
              <Loader2 className="w-3.5 h-3.5 animate-spin" />
            ) : (
              <Play className="w-3.5 h-3.5" />
            )}
            Test
          </button>
          {previewLatency !== null && (
            <span className="text-xs text-muted">
              {previewLatency < 5 ? "Stage 1 only" : `Cleaned in ${previewLatency}ms`}
              {previewModel && previewModel !== "none" && ` with ${previewModel}`}
            </span>
          )}
        </div>
        <div className="rounded-lg bg-surface-2 border border-line px-3 py-2.5">
          <p className="text-2xs text-muted mb-1">Output</p>
          <p className="text-sm text-ink leading-6">
            {previewOut || "Click Test to see the cleaned result."}
          </p>
        </div>
      </div>

      <Row label="Remove filler words" hint='Drops "um", "uh", "er", "hmm"'>
        <Toggle
          on={settings.filler_removal_enabled}
          onChange={(v) => onUpdateSettings({ filler_removal_enabled: v })}
          ariaLabel="Remove filler words"
        />
      </Row>
      <Row label="Spoken punctuation" hint='Say "period", "comma", "new line"'>
        <Toggle
          on={settings.spoken_punctuation_enabled}
          onChange={(v) => onUpdateSettings({ spoken_punctuation_enabled: v })}
          ariaLabel="Spoken punctuation"
        />
      </Row>
      <ApplicationProfiles settings={settings} onUpdateSettings={onUpdateSettings} />
    </Section>
  );
};
