import React, { useEffect, useMemo, useState } from "react";
import {
  AppSettings,
  FlowStatus,
  IntelligenceTier,
  IntelligenceTierState,
  INTELLIGENCE_TIERS,
  ModelStatus,
  RuntimeDownloadEvent,
  SystemMetrics,
  TranscriptStyle,
} from "../../types";
import { api } from "../../services/tauriApi";
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
  Zap,
  Check,
  AlertTriangle,
} from "lucide-react";
import type { IntelligenceDownloadEvent } from "../../App";

interface Props {
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => void;
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

const formatSize = (mb: number) =>
  mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${mb} MB`;

export const CleanupPage: React.FC<Props> = ({
  settings,
  onUpdateSettings,
  modelStatus,
  intelligenceDownload,
  activeDownloadTiers,
  runtimeDownload,
  runtimeDownloadActive,
  runtimeDownloadError,
  onInstallRuntime,
  onRemoveRuntime,
}) => {
  const [flowStatus, setFlowStatus] = useState<FlowStatus | null>(null);
  const [intelligenceTiers, setIntelligenceTiers] = useState<
    IntelligenceTierState[] | null
  >(null);
  const [systemMetrics, setSystemMetrics] = useState<SystemMetrics | null>(null);
  const [installedModels, setInstalledModels] = useState<Record<string, boolean>>({});
  const [installing, setInstalling] = useState<IntelligenceTier | null>(null);
  const [sample, setSample] = useState(
    "I want to drink coffee um no wait I want tea"
  );
  const [previewOut, setPreviewOut] = useState("");
  const [previewLatency, setPreviewLatency] = useState<number | null>(null);
  const [previewModel, setPreviewModel] = useState<string>("");
  const [previewing, setPreviewing] = useState(false);
  const [removing, setRemoving] = useState<IntelligenceTier | null>(null);

  const tier: IntelligenceTier = settings.intelligence_tier ?? "smart_flow";

  // Pull the live flow + system metrics so badges and warnings stay accurate.
  useEffect(() => {
    // Suspend polling while a download is in flight: the backend reports
    // `installed: false` and `is_downloading: false` until the file is fully
    // written, so polling during the download would clobber our UI state.
    const anyActive = activeDownloadTiers.size > 0;
    let alive = true;
    const refresh = async () => {
      try {
        const [fs, sm, tiers] = await Promise.all([
          api.getIntelligenceStatus(),
          api.getSystemMetrics(),
          api.getIntelligenceTiers(),
        ]);
        if (!alive) return;
        setFlowStatus(fs);
        setSystemMetrics(sm);
        setIntelligenceTiers(tiers);
      } catch (e) {
        console.error("intelligence status refresh failed", e);
      }
    };
    if (anyActive) {
      return () => {
        alive = false;
      };
    }
    refresh();
    const interval = setInterval(refresh, 1500);
    return () => {
      alive = false;
      clearInterval(interval);
    };
  }, [settings.flow_model, activeDownloadTiers]);

  // Refresh installed-model markers when a download completes.
  useEffect(() => {
    if (intelligenceDownload?.phase === "complete") {
      setInstalledModels((prev) => ({ ...prev, [intelligenceDownload.tier]: true }));
    }
  }, [intelligenceDownload]);

  const gpu = (modelStatus?.backend ?? systemMetrics?.gpu_name ?? "").toLowerCase();
  const hasGpu = /cuda|gpu|nvidia|metal|radeon/.test(gpu);
  const totalRamMb = systemMetrics?.total_ram_mb ?? 0;
  const vramMb = systemMetrics?.vram_mb ?? 0;
  const lowSpecPc = !hasGpu || totalRamMb < 8 * 1024;

  const recommendedTier: IntelligenceTier = useMemo(() => {
    if (vramMb >= 8 * 1024 || (hasGpu && totalRamMb >= 16 * 1024)) {
      return "deep_context";
    }
    if (totalRamMb < 8 * 1024 && !hasGpu) {
      return "raw_verbatim";
    }
    return "smart_flow";
  }, [vramMb, hasGpu, totalRamMb]);

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
    const ggufInstalled = installedModels[t] ?? false;
    if (!ggufInstalled) return false;
    return !runtimeInstalled;
  };

  const isDownloading = (t: IntelligenceTier) => activeDownloadTiers.has(t);

  const downloadProgress = (t: IntelligenceTier) =>
    isDownloading(t) && intelligenceDownload?.tier === t
      ? intelligenceDownload.progress_pct
      : 0;

  const downloadSpeed = (t: IntelligenceTier) =>
    isDownloading(t) && intelligenceDownload?.tier === t
      ? intelligenceDownload.speed_mbps
      : 0;

  const handleSelectTier = async (t: IntelligenceTier) => {
    onUpdateSettings({ intelligence_tier: t });
    try {
      const updated = await api.setIntelligenceTier(t);
      onUpdateSettings(updated);
    } catch (e) {
      console.error("setIntelligenceTier failed", e);
    }
  };

  const handleInstall = async (t: IntelligenceTier) => {
    if (t === "raw_verbatim") return;
    if (activeDownloadTiers.has(t)) return;
    if (tierState(t)?.installed) return;
    setInstalling(t);
    try {
      await api.installIntelligenceModel(t);
    } catch (e) {
      console.error("installIntelligenceModel failed", e);
    } finally {
      setTimeout(() => setInstalling(null), 500);
    }
  };

  const handleRemove = async (t: IntelligenceTier) => {
    if (t === "raw_verbatim") return;
    setRemoving(t);
    try {
      await api.removeIntelligenceModel(t);
      setInstalledModels((prev) => ({ ...prev, [t]: false }));
    } catch (e) {
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
      console.error("preview failed", e);
      setPreviewOut(sample);
      setPreviewLatency(0);
      setPreviewModel("none");
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
      <div className="grid grid-cols-1 gap-3">
        {(Object.keys(INTELLIGENCE_TIERS) as IntelligenceTier[]).map((id) => {
          const meta = INTELLIGENCE_TIERS[id];
          const active = tier === id;
          const installed = isInstalled(id);
          const downloading = isDownloading(id);
          const progress = downloadProgress(id);
          const speed = downloadSpeed(id);
          const isRemoving = removing === id;
          const isRecommended =
            id === recommendedTier && tier !== id;
          const isDeepContext = id === "deep_context";
          const needsGpu = isDeepContext && !hasGpu;
          return (
            <div
              key={id}
              className={`relative rounded-2xl border transition-all p-4 ${
                active
                  ? "border-accent bg-accent-soft shadow-xs ring-1 ring-accent"
                  : "border-line bg-surface hover:border-line-strong hover:bg-base-2"
              }`}
            >
              <div className="flex items-start gap-3">
                <button
                  type="button"
                  onClick={() => handleSelectTier(id)}
                  className="flex items-start gap-3 text-left flex-1 min-w-0"
                >
                  <div
                    className={`w-9 h-9 rounded-xl flex items-center justify-center shrink-0 ${
                      active
                        ? "bg-accent text-white"
                        : "bg-surface-2 text-ink-2"
                    }`}
                  >
                    {TIER_ICONS[id]}
                  </div>
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      <p
                        className={`text-[14px] font-semibold ${
                          active ? "text-accent" : "text-ink"
                        }`}
                      >
                        {meta.label}
                      </p>
                      <span
                        className={`px-1.5 py-0.5 rounded text-[9.5px] font-bold tracking-wider ${
                          id === "smart_flow"
                            ? "bg-emerald-500/15 text-emerald-600 dark:text-emerald-300"
                            : id === "deep_context"
                              ? "bg-violet-500/15 text-violet-600 dark:text-violet-300"
                              : "bg-slate-500/15 text-slate-600 dark:text-slate-300"
                        }`}
                      >
                        {meta.badgeText}
                      </span>
                      {isRecommended && (
                        <span className="px-1.5 py-0.5 rounded text-[9.5px] font-bold tracking-wider bg-amber-500/15 text-amber-700 dark:text-amber-300">
                          BEST FOR YOUR PC
                        </span>
                      )}
                    </div>
                    <p className="text-[11.5px] text-muted mt-0.5 leading-snug">
                      {meta.tagline}
                    </p>
                    <p className="text-[12.5px] text-ink-2 mt-2 leading-relaxed">
                      {meta.description}
                    </p>
                    <div className="flex flex-wrap gap-1.5 mt-2.5">
                      <span className="px-2 py-0.5 rounded-full bg-base-2 border border-line text-[10.5px] text-ink-2 font-medium">
                        {meta.latencyEstimate}
                      </span>
                      {meta.downloadSizeMB > 0 && (
                        <span className="px-2 py-0.5 rounded-full bg-base-2 border border-line text-[10.5px] text-ink-2 font-medium">
                          {formatSize(meta.downloadSizeMB)}
                        </span>
                      )}
                      {meta.ramRequiredMB > 0 && (
                        <span className="px-2 py-0.5 rounded-full bg-base-2 border border-line text-[10.5px] text-ink-2 font-medium">
                          {formatSize(meta.ramRequiredMB)} RAM
                        </span>
                      )}
                      {meta.vramRequiredMB > 0 && (
                        <span className="px-2 py-0.5 rounded-full bg-base-2 border border-line text-[10.5px] text-ink-2 font-medium">
                          {formatSize(meta.vramRequiredMB)} VRAM
                        </span>
                      )}
                      {id === "raw_verbatim" && (
                        <span className="px-2 py-0.5 rounded-full bg-base-2 border border-line text-[10.5px] text-ink-2 font-medium">
                          0 MB extra
                        </span>
                      )}
                      {id === "smart_flow" && (
                        <span className="px-2 py-0.5 rounded-full bg-base-2 border border-line text-[10.5px] text-ink-2 font-medium">
                          86% IFEval
                        </span>
                      )}
                      {id === "deep_context" && (
                        <span className="px-2 py-0.5 rounded-full bg-base-2 border border-line text-[10.5px] text-ink-2 font-medium">
                          201 Languages
                        </span>
                      )}
                    </div>
                    {needsGpu && (
                      <div className="mt-2.5 flex items-start gap-1.5 text-[11.5px] text-amber-700 dark:text-amber-300">
                        <AlertTriangle className="w-3.5 h-3.5 mt-0.5 shrink-0" />
                        <span>
                          {lowSpecPc
                            ? "Requires 4 GB+ free memory or a dedicated GPU. Slower on CPU-only laptops."
                            : "Best with a dedicated GPU. CPU inference will be slower."}
                        </span>
                      </div>
                    )}
                    <p className="text-[10.5px] text-muted mt-2 italic">
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
                      <span className="inline-flex items-center gap-1.5 text-[11.5px] text-emerald-600 dark:text-emerald-300 font-medium">
                        <ShieldCheck className="w-3.5 h-3.5" />
                        Installed
                      </span>
                      <div className="flex-1" />
                      <button
                        type="button"
                        className="btn btn-ghost !py-1.5 !px-3 !text-[12px]"
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
                      <div className="flex items-center justify-between text-[11.5px]">
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
                          className="h-full bg-accent transition-all duration-300 rounded-full"
                          style={{ width: `${progress}%` }}
                        />
                      </div>
                    </div>
                  ) : isWeightsOnly(id) ? (
                    <>
                      <span className="inline-flex items-center gap-1.5 text-[11.5px] text-amber-600 dark:text-amber-300 font-medium">
                        <AlertTriangle className="w-3.5 h-3.5" />
                        Weights only — runtime missing
                      </span>
                      <div className="flex-1" />
                      {runtimeDownloadActive ? (
                        <span className="inline-flex items-center gap-1.5 text-[11px] text-accent font-medium">
                          <Loader2 className="w-3 h-3 animate-spin" />
                          Installing runtime… {runtimeDownload?.progress_pct ?? 0}%
                        </span>
                      ) : (
                        <button
                          type="button"
                          className="btn btn-primary !py-1.5 !px-3 !text-[12px]"
                          onClick={onInstallRuntime}
                        >
                          <Cpu className="w-3.5 h-3.5" />
                          Install runtime
                        </button>
                      )}
                    </>
                  ) : (
                    <>
                      <span className="text-[11.5px] text-muted">
                        Download the GGUF weights to use this tier.
                      </span>
                      <div className="flex-1" />
                      <button
                        type="button"
                        className="btn btn-primary !py-1.5 !px-3 !text-[12px]"
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
              <summary className="cursor-pointer px-3.5 py-2.5 flex items-center justify-between text-[12.5px] font-semibold text-ink select-none">
                <span>Voice tone &amp; style</span>
                <span className="text-[11px] text-muted font-normal group-open:hidden">
                  Show options
                </span>
                <span className="text-[11px] text-muted font-normal hidden group-open:inline">
                  Hide
                </span>
              </summary>
              <div className="px-3.5 pb-3.5 space-y-3 border-t border-line/60">
                <Row label="Style" hint="Adjusts the tone of the rewrite">
                  <select
                    className="field"
                    value={settings.style ?? "neutral"}
                    onChange={(e) =>
                      onUpdateSettings({ style: e.target.value as TranscriptStyle })
                    }
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
          <p className="text-[12.5px] font-semibold text-ink">Live playground</p>
          <span className="text-[10.5px] text-muted">
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
            className="btn btn-primary !py-1.5 !px-3 !text-[12.5px]"
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
            <span className="text-[11.5px] text-muted">
              {previewLatency < 5
                ? "Stage 1 only"
                : `Cleaned in ${previewLatency}ms`}
              {previewModel && previewModel !== "none" && ` with ${previewModel}`}
            </span>
          )}
        </div>
        <div className="rounded-lg bg-surface-2 border border-line px-3 py-2.5">
          <p className="text-[11px] text-muted mb-1">Output</p>
          <p className="text-[13px] text-ink leading-6">
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
    </Section>
  );
};
