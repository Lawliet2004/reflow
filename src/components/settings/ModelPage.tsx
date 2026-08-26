import React, { useState } from "react";
import {
  AppSettings,
  ComputeBackend,
  FlowStatus,
  IntelligenceTier,
  IntelligenceTierState,
  INTELLIGENCE_TIERS,
  ModelStatus,
  RuntimeDownloadEvent,
  isModelLoading,
  isModelReady,
} from "../../types";
import { api } from "../../services/tauriApi";
import { Section, Row, Toggle } from "./ui";
import {
  Check,
  Cpu,
  Download,
  Globe,
  Loader2,
  RefreshCw,
  ShieldCheck,
  Sparkles,
  Trash2,
  Zap,
  AlertTriangle,
} from "lucide-react";
import type { IntelligenceDownloadEvent } from "../../App";

interface Props {
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => void;
  modelStatus: ModelStatus | null;
  onReloadModel: () => void;
  intelligenceDownload: IntelligenceDownloadEvent | null;
  activeDownloadTiers: Set<IntelligenceTier>;
  runtimeDownload: RuntimeDownloadEvent | null;
  runtimeDownloadActive: boolean;
  runtimeDownloadError: string | null;
  onInstallRuntime: () => void;
  onRemoveRuntime: () => void;
}

const FORMAT_SIZE = (mb: number) =>
  mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${mb} MB`;

const MODELS: { id: "0.6b" | "1.7b"; title: string; desc: string }[] = [
  { id: "0.6b", title: "0.6B · Realtime", desc: "Instant latency · fits all GPUs & CPU" },
  { id: "1.7b", title: "1.7B · High Accuracy", desc: "Maximum precision · auto-fits 4 GB+ GPUs" },
];

type Precision = "auto" | "int4" | "int8" | "bf16";
const PRECISIONS: {
  id: Precision;
  title: string;
  desc: string;
}[] = [
  { id: "auto", title: "Auto", desc: "Picks the best fit for your GPU" },
  { id: "int4", title: "4-bit", desc: "Lowest VRAM · small accuracy hit" },
  { id: "int8", title: "8-bit", desc: "Half the VRAM · near-lossless" },
  { id: "bf16", title: "16-bit", desc: "Full precision · needs the most VRAM" },
];

export const ModelPage: React.FC<Props> = ({
  settings,
  onUpdateSettings,
  modelStatus,
  onReloadModel,
  intelligenceDownload,
  activeDownloadTiers,
  runtimeDownload,
  runtimeDownloadActive,
  runtimeDownloadError,
  onInstallRuntime,
  onRemoveRuntime,
}) => {
  const [installingModel, setInstallingModel] = useState<string | null>(null);
  const [removing, setRemoving] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [flowStatus, setFlowStatus] = useState<FlowStatus | null>(null);
  const [intelligenceTiers, setIntelligenceTiers] = useState<
    IntelligenceTierState[] | null
  >(null);
  const [installingIntelligence, setInstallingIntelligence] =
    useState<IntelligenceTier | null>(null);
  const [removingIntelligence, setRemovingIntelligence] =
    useState<IntelligenceTier | null>(null);
  const [confirmRemoveIntelligence, setConfirmRemoveIntelligence] =
    useState<IntelligenceTier | null>(null);

  React.useEffect(() => {
    // While a download is running, the on-disk `installed` flag is always false
    // and the backend reports `is_downloading: false` until the file is fully
    // written. Polling during that window would clobber our UI state and keep
    // the Download button visible. Suspend the poll until every active
    // download has reached a terminal phase.
    const anyActive = activeDownloadTiers.size > 0;
    let alive = true;
    const refresh = async () => {
      try {
        const [status, tiers] = await Promise.all([
          api.getIntelligenceStatus(),
          api.getIntelligenceTiers(),
        ]);
        if (!alive) return;
        setFlowStatus(status);
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
    const interval = setInterval(refresh, 2000);
    return () => {
      alive = false;
      clearInterval(interval);
    };
  }, [settings.flow_model, activeDownloadTiers]);

  const tierState = (tier: IntelligenceTier): IntelligenceTierState | null => {
    if (tier === "raw_verbatim") {
      return {
        tier,
        model_id: "none",
        installed: true,
        downloading: false,
      };
    }
    return intelligenceTiers?.find((t) => t.tier === tier) ?? null;
  };

  const isIntelligenceInstalled = (tier: IntelligenceTier) =>
    tierState(tier)?.installed ?? false;

  const isIntelligenceDownloading = (tier: IntelligenceTier) =>
    activeDownloadTiers.has(tier);

  const handleInstallIntelligence = async (tier: IntelligenceTier) => {
    if (tier === "raw_verbatim") return;
    if (activeDownloadTiers.has(tier)) return;
    if (tierState(tier)?.installed) return;
    setInstallingIntelligence(tier);
    try {
      await api.installIntelligenceModel(tier);
    } catch (e) {
      console.error("installIntelligenceModel failed", e);
    } finally {
      setTimeout(() => setInstallingIntelligence(null), 500);
    }
  };

  const handleRemoveIntelligence = async (tier: IntelligenceTier) => {
    if (tier === "raw_verbatim") return;
    setRemovingIntelligence(tier);
    try {
      await api.removeIntelligenceModel(tier);
    } catch (e) {
      console.error("removeIntelligenceModel failed", e);
    } finally {
      setRemovingIntelligence(null);
      setConfirmRemoveIntelligence(null);
    }
  };

  const change = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) =>
    onUpdateSettings({ [key]: value } as Partial<AppSettings>);

  const handleSelectOffload = (value: number) =>
    onUpdateSettings({ flow_n_gpu_layers: value } as Partial<AppSettings>);

  const handleSelectModel = async (id: "0.6b" | "1.7b") => {
    if (settings.asr_model === id) return;
    await change("asr_model", id);
    try {
      const status = await api.getModelStatus();
      if (!status.installed) {
        setInstallingModel(id);
        await api.installModel(id);
      } else {
        await api.reloadModel();
      }
    } catch (e) {
      console.error("Model select error:", e);
    } finally {
      setTimeout(() => setInstallingModel(null), 2000);
    }
  };

  const handleSelectPrecision = async (id: Precision) => {
    if (settings.asr_precision === id) return;
    await change("asr_precision", id);
    // The choice is already persisted to AppSettings, so a future launch
    // will honor it. Reload now so the new precision is live without
    // requiring a restart.
    if (modelStatus?.installed) {
      try {
        await api.reloadModel();
      } catch (e) {
        console.error("Precision reload error:", e);
      }
    }
  };

  const removeModel = async () => {
    setRemoving(true);
    try {
      await api.removeModel(settings.asr_model);
    } catch (e) {
      console.error("Remove model error:", e);
    }
    setRemoving(false);
    setConfirmRemove(false);
  };

  const backendLabel = modelStatus?.backend ?? "";
  const modelReady = isModelReady(modelStatus);
  const onGpu = /cuda|gpu/i.test(backendLabel);
  const downloading = Boolean(modelStatus?.is_downloading);
  const loading = isModelLoading(modelStatus);

  // Parse the active mode out of the sidecar's backend string.
  // Format: "Qwen3-ASR 1.7B · CUDA int8" / "Qwen3-ASR 1.7B · CPU cpu"
  // Falls back to "—" when nothing useful is reported.
  const precisionLabel = (() => {
    if (!modelReady) return null;
    const m = backendLabel.match(/(CUDA|CPU)\s+(\S+)/i);
    if (!m) return null;
    const device = m[1].toUpperCase();
    const mode = m[2].toLowerCase();
    const vram = modelStatus && "vram_mb" in modelStatus ? (modelStatus as { vram_mb?: number }).vram_mb : undefined;
    const vramText = vram && vram > 0 ? ` · ${vram.toFixed(0)} MB VRAM` : "";
    return `Currently: ${device} ${mode}${vramText}`;
  })();

  const showGpuHint =
    Boolean(modelStatus?.gpu_available) &&
    modelStatus?.cuda_available === false &&
    Boolean(modelStatus?.asr_gpu_hint);

  return (
    <Section
      icon={<Cpu className="w-4 h-4" />}
      title="Speech model"
    >
      {showGpuHint && (
        <div
          className="rounded-xl border border-amber-500/40 bg-amber-500/10 p-3 space-y-1.5"
          role="status"
        >
          <div className="flex items-start gap-2 text-[12.5px] text-amber-700 dark:text-amber-300">
            <AlertTriangle className="w-4 h-4 mt-0.5 shrink-0" />
            <div>
              <p className="font-semibold">
                GPU detected, but ASR is using CPU.
              </p>
              <p className="mt-1 leading-snug">
                Your system Python&rsquo;s <code>torch</code> was installed
                without CUDA support. Run the command below in a terminal, then
                click <em>Reload speech model</em>:
              </p>
            </div>
          </div>
          <pre className="text-[11.5px] font-mono bg-surface-2 border border-amber-500/30 rounded-md px-2 py-1.5 overflow-x-auto whitespace-pre">
            {modelStatus?.asr_gpu_hint}
          </pre>
        </div>
      )}
      <div className="grid grid-cols-2 gap-3">
        {MODELS.map((m) => {
          const active = settings.asr_model === m.id;
          return (
            <button
              key={m.id}
              onClick={() => handleSelectModel(m.id)}
              className={`text-left rounded-xl border p-3.5 transition-all cursor-pointer ${
                active
                  ? "border-accent bg-accent-soft shadow-xs ring-1 ring-accent"
                  : "border-line bg-surface hover:border-line-strong hover:bg-base-2"
              }`}
            >
              <div className="flex items-center justify-between">
                <p
                  className={`text-[13px] font-semibold ${
                    active ? "text-accent" : "text-ink"
                  }`}
                >
                  {m.title}
                </p>
                {active && (
                  <div className="w-4 h-4 rounded-full bg-accent text-white flex items-center justify-center">
                    <Check className="w-2.5 h-2.5 stroke-[3]" />
                  </div>
                )}
              </div>
              <p className="text-[11.5px] text-muted mt-1 leading-snug">{m.desc}</p>
              {installingModel === m.id && (
                <p className="text-[11px] text-accent mt-2 flex items-center gap-1.5 font-medium">
                  <Loader2 className="w-3 h-3 animate-spin" />
                  Downloading{" "}
                  {modelStatus?.is_downloading && active
                    ? `${modelStatus.download_progress_pct}%`
                    : "…"}
                </p>
              )}
            </button>
          );
        })}
      </div>

      <div
        className={`rounded-xl border p-4 transition-all ${
          modelReady
            ? "border-accent-border bg-accent-soft/60"
            : downloading || loading
              ? "border-line bg-surface-2"
              : modelStatus?.error
                ? "border-rose-500/30 bg-rose-500/10"
                : "border-line bg-surface-2"
        }`}
      >
        <div className="flex items-center justify-between gap-3">
          <div className="flex items-center gap-2.5 min-w-0">
            {modelReady ? (
              onGpu ? (
                <div className="w-8 h-8 rounded-lg bg-accent-soft border border-accent-border text-accent flex items-center justify-center shrink-0">
                  <Zap className="w-4 h-4" />
                </div>
              ) : (
                <div className="w-8 h-8 rounded-lg bg-surface-3 text-ink-2 flex items-center justify-center shrink-0">
                  <Cpu className="w-4 h-4" />
                </div>
              )
            ) : downloading || loading ? (
              <div className="w-8 h-8 rounded-lg bg-accent-soft border border-accent-border text-accent flex items-center justify-center shrink-0">
                <Loader2 className="w-4 h-4 animate-spin" />
              </div>
            ) : (
              <div className="w-8 h-8 rounded-lg bg-surface-3 text-muted flex items-center justify-center shrink-0">
                <Cpu className="w-4 h-4" />
              </div>
            )}
            <div className="min-w-0">
              <p className="text-[13px] font-semibold text-ink truncate">
                {modelStatus?.name || "Qwen3-ASR Engine"}
              </p>
              <p className="text-[11.5px] text-muted truncate">
                {modelReady
                  ? modelStatus?.backend
                  : downloading
                    ? `Downloading weights · ${modelStatus?.download_progress_pct ?? 0}%`
                    : loading
                      ? `${
                          backendLabel && !/loading/i.test(backendLabel)
                            ? backendLabel
                            : `Loading ${settings.asr_model === "1.7b" ? "1.7B" : "0.6B"} model`
                        }`
                      : modelStatus?.error ?? "Initializing model…"}
              </p>
              {precisionLabel && (
                <p className="text-[10.5px] text-accent mt-0.5 font-medium">
                  {precisionLabel}
                </p>
              )}
            </div>
          </div>
          {modelStatus?.installed && (
            <button
              className="icon-btn hover:bg-surface-2 hover:text-accent shadow-xs border border-line"
              title="Reload speech model"
              aria-label="Reload speech model"
              onClick={onReloadModel}
            >
              <RefreshCw className="w-3.5 h-3.5" />
            </button>
          )}
        </div>

        {downloading && (
          <div className="h-1.5 rounded-full bg-line mt-3 overflow-hidden">
            <div
              className="h-full bg-accent transition-all duration-500 rounded-full"
              style={{ width: `${modelStatus?.download_progress_pct ?? 0}%` }}
            />
          </div>
        )}

        {modelStatus?.error && (
          <p className="text-[11.5px] text-rose-500 mt-2 leading-relaxed font-medium">
            {modelStatus.error}
          </p>
        )}
      </div>

      <Row label="Compute backend" hint="Auto uses CUDA when a compatible GPU is present">
        <select
          className="field"
          value={settings.compute_backend}
          onChange={(e) => change("compute_backend", e.target.value as ComputeBackend)}
        >
          <option value="auto">Auto (GPU prioritized)</option>
          <option value="gpu">GPU only (CUDA)</option>
          <option value="cpu">CPU only</option>
        </select>
      </Row>

      <div>
        <div className="flex items-center justify-between gap-6 mb-1.5">
          <div className="min-w-0">
            <p className="text-[13px] text-ink font-medium">Model precision</p>
            <p className="text-[11.5px] text-muted mt-0.5 leading-relaxed">
              Lower precision uses less VRAM; 16-bit is the most accurate. The
              choice is remembered across launches.
            </p>
          </div>
        </div>
        <div className="grid grid-cols-4 gap-2">
          {PRECISIONS.map((p) => {
            const active = settings.asr_precision === p.id;
            return (
              <button
                key={p.id}
                onClick={() => handleSelectPrecision(p.id)}
                className={`text-left rounded-lg border p-2.5 transition-all cursor-pointer ${
                  active
                    ? "border-accent bg-accent-soft shadow-xs ring-1 ring-accent"
                    : "border-line bg-surface hover:border-line-strong hover:bg-base-2"
                }`}
              >
                <div className="flex items-center justify-between">
                  <p
                    className={`text-[12.5px] font-semibold ${
                      active ? "text-accent" : "text-ink"
                    }`}
                  >
                    {p.title}
                  </p>
                  {active && (
                    <div className="w-3.5 h-3.5 rounded-full bg-accent text-white flex items-center justify-center">
                      <Check className="w-2 h-2 stroke-[3]" />
                    </div>
                  )}
                </div>
                <p className="text-[10.5px] text-muted mt-0.5 leading-snug">
                  {p.desc}
                </p>
              </button>
            );
          })}
        </div>
      </div>

      <Row label="Keep model loaded" hint="Pre-warms the model in memory at startup">
        <Toggle
          on={settings.keep_model_loaded}
          onChange={(v) => change("keep_model_loaded", v)}
          ariaLabel="Keep model loaded"
        />
      </Row>

      {modelStatus?.installed && !confirmRemove && (
        <button
          className="btn btn-danger w-full mt-2"
          onClick={() => setConfirmRemove(true)}
        >
          <Trash2 className="w-3.5 h-3.5" />
          Remove downloaded model weights
        </button>
      )}
      {modelStatus?.installed && confirmRemove && (
        <div className="p-3 rounded-xl border border-rose-500/30 bg-rose-500/10 space-y-2">
          <div className="flex items-start gap-2 text-[12.5px] text-rose-700 dark:text-rose-300">
            <AlertTriangle className="w-4 h-4 mt-0.5 shrink-0" />
            <p>
              This deletes the {settings.asr_model.toUpperCase()} weights from your computer. You'll
              need to re-download to dictate again.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <button
              className="btn btn-danger !py-1.5 !px-3 !text-[12.5px]"
              onClick={removeModel}
              disabled={removing}
            >
              <Trash2 className="w-3.5 h-3.5" />
              {removing ? "Removing…" : "Yes, remove"}
            </button>
            <button
              className="btn btn-ghost !py-1.5 !px-3 !text-[12.5px]"
              onClick={() => setConfirmRemove(false)}
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      <div className="pt-4 border-t border-line space-y-3">
        <div className="flex items-center gap-2">
          <Sparkles className="w-4 h-4 text-accent" />
          <h3 className="text-[13.5px] font-semibold text-ink">
            Intelligence &amp; Flow Engine
          </h3>
        </div>
        <p className="text-[12px] text-muted -mt-1">
          Stage 2 LLM post-processing. Download the GGUF weights for the tier you want to
          enable.
        </p>

        {(["smart_flow", "deep_context"] as IntelligenceTier[]).map((tier) => {
          const meta = INTELLIGENCE_TIERS[tier];
          const state = tierState(tier);
          const ggufInstalled = state !== null && state.installed;
          const runtimeInstalled = flowStatus?.runtime_installed ?? false;
          const installed = ggufInstalled && runtimeInstalled;
          const showRuntimeMissing = ggufInstalled && !runtimeInstalled;
          const downloading = isIntelligenceDownloading(tier);
          const isActive = settings.intelligence_tier === tier;
          const isRemoving = removingIntelligence === tier;
          const icon =
            tier === "smart_flow" ? (
              <Sparkles className="w-4 h-4" />
            ) : (
              <Globe className="w-4 h-4" />
            );
          return (
            <div
              key={tier}
              className={`rounded-xl border p-4 space-y-3 ${
                isActive
                  ? "border-accent bg-accent-soft"
                  : "border-line bg-surface"
              }`}
            >
              <div className="flex items-start gap-3">
                <div
                  className={`w-9 h-9 rounded-lg flex items-center justify-center shrink-0 ${
                    isActive ? "bg-accent text-white" : "bg-surface-2 text-ink-2"
                  }`}
                >
                  {icon}
                </div>
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2 flex-wrap">
                    <p className="text-[13px] font-semibold text-ink">
                      {meta.label}
                    </p>
                    {isActive && (
                      <span className="text-[9.5px] font-bold tracking-wider px-1.5 py-0.5 rounded bg-accent text-white">
                        ACTIVE
                      </span>
                    )}
                  </div>
                  <p className="text-[11.5px] text-muted mt-0.5 break-all">
                    {meta.modelFile}
                  </p>
                </div>
              </div>

              <div className="flex items-center justify-between gap-3 text-[11.5px] text-muted">
                <span>
                  <span className="text-ink font-medium">
                    {FORMAT_SIZE(meta.downloadSizeMB)}
                  </span>{" "}
                  download ·{" "}
                  <span className="text-ink font-medium">
                    {FORMAT_SIZE(meta.ramRequiredMB)}
                  </span>{" "}
                  RAM
                </span>
                {installed && (
                  <span className="inline-flex items-center gap-1 text-emerald-600 dark:text-emerald-300 font-semibold">
                    <ShieldCheck className="w-3.5 h-3.5" />
                    Installed
                  </span>
                )}
                {showRuntimeMissing && (
                  <span className="inline-flex items-center gap-1 text-amber-600 dark:text-amber-300 font-semibold">
                    <AlertTriangle className="w-3.5 h-3.5" />
                    Weights only
                  </span>
                )}
                {downloading && (
                  <span className="inline-flex items-center gap-1 text-accent font-semibold">
                    <Loader2 className="w-3.5 h-3.5 animate-spin" />
                    Downloading
                  </span>
                )}
                {runtimeDownloadActive && !installed && !downloading && (
                  <span className="inline-flex items-center gap-1 text-accent font-semibold">
                    <Loader2 className="w-3.5 h-3.5 animate-spin" />
                    Installing runtime
                  </span>
                )}
              </div>

              {installed && flowStatus?.ready && isActive && (
                <div className="text-[11px] text-muted bg-base-2/60 border border-line rounded-md px-2.5 py-1.5 leading-snug">
                  Loaded on {flowStatus.backend || "runtime"}
                  {flowStatus.n_gpu_layers !== undefined && (
                    <>
                      {" "}· {flowStatus.n_gpu_layers}
                      {flowStatus.mode === "gpu" ? "/99 layers" : " layers"}
                      {flowStatus.mode === "gpu" && flowStatus.vram_used_mb
                        ? ` · ${flowStatus.vram_used_mb.toFixed(0)} MB VRAM`
                        : ""}
                    </>
                  )}
                </div>
              )}

              {showRuntimeMissing && (
                <p className="text-[11px] text-amber-600 dark:text-amber-300 leading-snug">
                  Weights are downloaded, but the <code>llama-server</code>{" "}
                  runtime is missing from{" "}
                  <code>~/AppData/Roaming/reflow/bin/</code>. The tier can&rsquo;t
                  run until the runtime is installed.
                </p>
              )}
              {downloading && (
                <div className="space-y-1">
                  <div className="flex items-center justify-between text-[11px] text-muted">
                    <span>Downloading weights</span>
                    <span>
                      {intelligenceDownload?.progress_pct ?? 0}% ·{" "}
                      {(intelligenceDownload?.speed_mbps ?? 0).toFixed(1)} MB/s
                    </span>
                  </div>
                  <div className="h-1.5 rounded-full bg-line overflow-hidden">
                    <div
                      className="h-full bg-accent transition-all duration-300 rounded-full"
                      style={{
                        width: `${intelligenceDownload?.progress_pct ?? 0}%`,
                      }}
                    />
                  </div>
                </div>
              )}
              {runtimeDownloadActive && runtimeDownload && (
                <div className="space-y-1">
                  <div className="flex items-center justify-between text-[11px] text-muted">
                    <span>
                      Installing {runtimeDownload.kind_label ?? "runtime"} runtime
                    </span>
                    <span>
                      {runtimeDownload.progress_pct}% ·{" "}
                      {runtimeDownload.speed_mbps.toFixed(1)} MB/s
                    </span>
                  </div>
                  <div className="h-1.5 rounded-full bg-line overflow-hidden">
                    <div
                      className="h-full bg-accent transition-all duration-300 rounded-full"
                      style={{ width: `${runtimeDownload.progress_pct}%` }}
                    />
                  </div>
                  {runtimeDownloadError && (
                    <p className="text-[10.5px] text-rose-600 dark:text-rose-300 leading-snug">
                      {runtimeDownloadError}
                    </p>
                  )}
                </div>
              )}
              {showRuntimeMissing && !runtimeDownloadActive && runtimeDownloadError && (
                <p className="text-[10.5px] text-rose-600 dark:text-rose-300 leading-snug">
                  Last install attempt failed: {runtimeDownloadError}
                </p>
              )}
              {confirmRemoveIntelligence === tier && (
                <div className="p-2.5 rounded-lg border border-rose-500/30 bg-rose-500/10 space-y-1.5">
                  <p className="text-[11.5px] text-rose-700 dark:text-rose-300">
                    Delete the downloaded weights for {meta.label}? You can re-download
                    later.
                  </p>
                  <div className="flex items-center gap-2">
                    <button
                      className="btn btn-danger !py-1 !px-2.5 !text-[11.5px]"
                      onClick={() => handleRemoveIntelligence(tier)}
                      disabled={isRemoving}
                    >
                      <Trash2 className="w-3 h-3" />
                      {isRemoving ? "Removing…" : "Yes, remove"}
                    </button>
                    <button
                      className="btn btn-ghost !py-1 !px-2.5 !text-[11.5px]"
                      onClick={() => setConfirmRemoveIntelligence(null)}
                    >
                      Cancel
                    </button>
                  </div>
                </div>
              )}

              <div className="flex items-center justify-end gap-2 pt-1 border-t border-line/60">
                {installed ? (
                  <button
                    className="btn btn-ghost !py-1 !px-2.5 !text-[11.5px]"
                    onClick={() => setConfirmRemoveIntelligence(tier)}
                    disabled={isRemoving}
                  >
                    <Trash2 className="w-3 h-3" />
                    Remove
                  </button>
                ) : showRuntimeMissing ? (
                  <>
                    <button
                      className="btn btn-primary !py-1 !px-2.5 !text-[11.5px]"
                      onClick={onInstallRuntime}
                      disabled={runtimeDownloadActive}
                    >
                      {runtimeDownloadActive ? (
                        <Loader2 className="w-3 h-3 animate-spin" />
                      ) : (
                        <Cpu className="w-3 h-3" />
                      )}
                      Install runtime
                    </button>
                    <button
                      className="btn btn-ghost !py-1 !px-2.5 !text-[11.5px]"
                      onClick={() => setConfirmRemoveIntelligence(tier)}
                      disabled={isRemoving}
                    >
                      <Trash2 className="w-3 h-3" />
                      Remove
                    </button>
                  </>
                ) : downloading || runtimeDownloadActive ? null : (
                  <button
                    className="btn btn-primary !py-1 !px-2.5 !text-[11.5px]"
                    onClick={() => handleInstallIntelligence(tier)}
                    disabled={installingIntelligence === tier}
                  >
                    {installingIntelligence === tier ? (
                      <Loader2 className="w-3 h-3 animate-spin" />
                    ) : (
                      <Download className="w-3 h-3" />
                    )}
                    Download
                  </button>
                )}
              </div>
            </div>
          );
        })}

        <div className="pt-1 space-y-2">
          <div className="flex items-center gap-2">
            <Cpu className="w-3.5 h-3.5 text-muted" />
            <p className="text-[12.5px] text-ink font-medium">GPU offload</p>
          </div>
          <p className="text-[11.5px] text-muted leading-snug">
            Choose how many transformer layers of the Stage 2 LLM run on the
            GPU. Lower this if you see OOMs or want to leave VRAM for the
            speech model. -1 = auto (binary 0/99 driven by Compute backend).
          </p>
          <div className="flex items-center gap-2 flex-wrap">
            <select
              className="field !py-1.5 !text-[12.5px]"
              value={offloadPresetValue(settings.flow_n_gpu_layers)}
              onChange={(e) => handleSelectOffload(Number(e.target.value))}
            >
              <option value="-1">Auto (-1)</option>
              <option value="0">CPU only (0)</option>
              <option value="30">Half (30)</option>
              <option value="99">Full GPU (99)</option>
            </select>
            <div className="flex items-center gap-1.5">
              <span className="text-[11px] text-muted">Custom:</span>
              <input
                type="number"
                min={-1}
                max={99}
                step={1}
                value={settings.flow_n_gpu_layers}
                onChange={(e) => {
                  const raw = Number(e.target.value);
                  if (Number.isNaN(raw)) return;
                  handleSelectOffload(Math.max(-1, Math.min(99, Math.floor(raw))));
                }}
                className="field !py-1.5 !text-[12.5px] w-20 text-center"
                aria-label="Custom GPU offload layer count"
              />
            </div>
            {settings.flow_n_gpu_layers >= 0 && (
              <span className="text-[10.5px] text-amber-600 dark:text-amber-300">
                Runtime will restart on the next dictation.
              </span>
            )}
          </div>
        </div>
      </div>
    </Section>
  );
};

function offloadPresetValue(v: number): string {
  if (v === -1 || v === 0 || v === 30 || v === 99) return String(v);
  return "custom";
}
