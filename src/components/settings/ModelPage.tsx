import React, { useState } from "react";
import {
  AppSettings,
  FlowModel,
  AsrSettings,
  Capabilities,
  DownloadedModel,
  IntelligenceTier,
  IntelligenceTierState,
  INTELLIGENCE_TIERS,
  ModelStatus,
  RuntimeDownloadEvent,
  primaryGpu,
  isModelLoading,
  isModelReady,
} from "../../types";
import { api } from "../../services/tauriApi";
import { ModelNotice } from "../ModelNotice";
import { PresetSelector } from "./PresetSelector";
import { RuntimeInventoryPanel } from "./RuntimeInventoryPanel";
import { CalibrationPanel } from "./CalibrationPanel";
import { Section, Row, Toggle } from "./ui";
import {
  AlertTriangle,
  Check,
  Cpu,
  Download,
  HardDrive,
  Loader2,
  RefreshCw,
  SlidersHorizontal,
  Sparkles,
  Trash2,
  Wrench,
} from "lucide-react";
import type { IntelligenceDownloadEvent } from "../../App";
import type { IntelligenceHub } from "../../hooks/useIntelligenceHub";
import { llmSelectionPatch, selectedLlm } from "../../llmSelection";

interface Props {
  intelligence: IntelligenceHub;
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => Promise<boolean>;
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

const FORMAT_SIZE = (mb: number) => (mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${mb} MB`);

const formatBytes = (bytes: number) =>
  bytes >= 1_000_000_000
    ? `${(bytes / 1_000_000_000).toFixed(1)} GB`
    : bytes >= 1_000_000
      ? `${Math.round(bytes / 1_000_000)} MB`
      : `${Math.ceil(bytes / 1000)} KB`;

const DOWNLOADED_KIND_LABEL: Record<DownloadedModel["kind"], string> = {
  asr: "Speech",
  llm: "Writing",
  runtime: "Runtime",
};

const MODELS: { id: string; title: string; desc: string }[] = [
  {
    id: "0.6b",
    title: "0.6B · Faster",
    desc: "Start here for everyday dictation and quick messages",
  },
  {
    id: "1.7b",
    title: "1.7B · Higher accuracy",
    desc: "Try for difficult names and mixed-language speech · more memory",
  },
  {
    id: "phonon-2",
    title: "Phonon-2 · Fast English",
    desc: "English only · no recognition hotword hints",
  },
  {
    id: "zipformer-20m",
    title: "Zipformer 20M INT8",
    desc: "English only · streaming · CPU-optimized",
  },
];

/** Approximate download sizes per runtime, shown before the files exist. */
const DOWNLOAD_SIZE: Record<string, { python: string; native: string }> = {
  "0.6b": { python: "1.6 GB", native: "1.0 GB" },
  "1.7b": { python: "4.1 GB", native: "2.5 GB" },
  "phonon-2": { python: "164 MB", native: "164 MB" },
  "zipformer-20m": { python: "44 MB", native: "44 MB" },
};

type Precision = "auto" | "int4" | "int8" | "bf16";
const PRECISIONS: {
  id: Precision;
  title: string;
  desc: string;
}[] = [
  { id: "auto", title: "Auto", desc: "Picks the best fit for your GPU" },
  { id: "int4", title: "4-bit", desc: "Lowest weight memory · measure speed and quality" },
  { id: "int8", title: "8-bit", desc: "Lower weight memory · speed varies by hardware" },
  { id: "bf16", title: "16-bit", desc: "Full precision · needs the most VRAM" },
];

const ICON_PROPS = { strokeWidth: 1.75, "aria-hidden": true } as const;

export const ModelPage: React.FC<Props> = ({
  settings,
  intelligence,
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
  const { flowStatus, intelligenceTiers } = intelligence;
  const [selectingLlm, setSelectingLlm] = useState(false);
  const selectLlm = async (model: FlowModel) => {
    setSelectingLlm(true);
    try {
      await onUpdateSettings(llmSelectionPatch(settings, model));
    } catch {
      intelligence.notifyToast("error", "The LLM choice could not be saved. Try again.");
    } finally {
      setSelectingLlm(false);
    }
  };
  const [capabilities, setCapabilities] = useState<Capabilities | null>(null);
  const [capabilitiesError, setCapabilitiesError] = useState<string | null>(null);

  const [installingIntelligence, setInstallingIntelligence] = useState<IntelligenceTier | null>(
    null,
  );

  const [downloaded, setDownloaded] = useState<DownloadedModel[] | null>(null);
  // One confirm strip and one in-flight delete across the whole page, keyed
  // `${kind}:${id}` — "asr" rows use the concrete on-disk id, "llm" the tier,
  // "runtime" the binary id.
  const [confirmDelete, setConfirmDelete] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);

  const refreshDownloaded = React.useCallback(async () => {
    try {
      setDownloaded(await api.getDownloadedModels());
    } catch (e) {
      console.error("getDownloadedModels failed", e);
    }
  }, []);

  React.useEffect(() => {
    let alive = true;
    api
      .refreshCapabilities()
      .then((next) => {
        if (!alive) return;
        setCapabilities(next);
        setCapabilitiesError(null);
      })
      .catch((error) => {
        if (!alive) return;
        console.error("hardware capability refresh failed", error);
        setCapabilitiesError("Hardware detection failed");
      });
    return () => {
      alive = false;
    };
  }, []);

  // Re-scan when no download is in flight so finished installs appear and a
  // mid-download refresh can't resurrect a row that's still being written.
  const anyDownloadActive =
    Boolean(modelStatus?.is_downloading) || activeDownloadTiers.size > 0 || runtimeDownloadActive;
  React.useEffect(() => {
    if (!anyDownloadActive) queueMicrotask(() => refreshDownloaded());
  }, [anyDownloadActive, refreshDownloaded]);

  const deleteModel = async (kind: DownloadedModel["kind"], id: string, label: string) => {
    const key = `${kind}:${id}`;
    setDeleting(key);
    try {
      if (kind === "asr") await api.removeModel(id);
      else if (kind === "llm") await api.removeIntelligenceModel(id as IntelligenceTier);
      else await onRemoveRuntime();
      await Promise.allSettled([refreshDownloaded(), intelligence.refresh()]);
    } catch (e) {
      intelligence.notifyToast("error", `Couldn't delete ${label}: ${String(e)}`);
      console.error("Delete model failed:", e);
    } finally {
      setDeleting(null);
      setConfirmDelete(null);
    }
  };

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

  const isIntelligenceDownloading = (tier: IntelligenceTier) => activeDownloadTiers.has(tier);

  const handleInstallIntelligence = async (tier: IntelligenceTier) => {
    if (tier === "raw_verbatim") return;
    if (activeDownloadTiers.has(tier)) return;
    if (tierState(tier)?.installed) return;
    setInstallingIntelligence(tier);
    try {
      await api.installIntelligenceModel(tier);
    } catch (e) {
      intelligence.notifyToast(
        "error",
        `Couldn't download ${INTELLIGENCE_TIERS[tier].label}: ${String(e)}`,
      );
      console.error("installIntelligenceModel failed", e);
    } finally {
      setInstallingIntelligence(null);
    }
  };

  const detectedGpu = capabilities ? primaryGpu(capabilities) : null;
  const refinementGpuAvailable = Boolean(detectedGpu);
  const refinementDevice =
    refinementGpuAvailable &&
    settings.refinement.device !== "cpu" &&
    settings.memory_policy.allow_gpu_refinement
      ? "gpu"
      : "cpu";

  const change = <K extends keyof AppSettings>(key: K, value: AppSettings[K]) =>
    onUpdateSettings({
      [key]: value,
      ...(key === "asr" ? { preset: "custom" } : {}),
    } as Partial<AppSettings>);

  const handleSelectRefinementDevice = (device: "cpu" | "gpu") => {
    if (device === "gpu" && !refinementGpuAvailable) return;
    onUpdateSettings({
      preset: "custom",
      refinement: {
        ...settings.refinement,
        device: device === "gpu" ? "vulkan" : "cpu",
        // GPU layer count is intentionally internal. -1 asks the backend to
        // choose the highest safe full or partial offload for current free VRAM.
        gpu_layers: device === "gpu" ? -1 : 0,
      },
      memory_policy: {
        ...settings.memory_policy,
        allow_gpu_refinement: device === "gpu",
      },
    });
  };

  const phononSelected = settings.asr.model === "phonon-2";
  const zipformerSelected = settings.asr.model === "zipformer-20m";
  const pythonOnlySelected = phononSelected || zipformerSelected;
  const handleSelectModel = async (id: string) => {
    if (settings.asr.model === id) return;
    if (
      !(await change("asr", {
        ...settings.asr,
        model: id,
        ...(id === "phonon-2" || id === "zipformer-20m"
          ? { runtime: "python" as const, precision: "auto" as const }
          : pythonOnlySelected
            ? { precision: "auto" as const }
            : {}),
      }))
    )
      return;
    const title = MODELS.find((m) => m.id === id)?.title ?? id;
    try {
      const status = await api.getModelStatus();
      if (!status.installed) {
        setInstallingModel(id);
        await api.installModel(id);
      } else {
        await api.reloadModel();
      }
    } catch (e) {
      intelligence.notifyToast("error", `Couldn't switch to ${title}: ${String(e)}`);
      console.error("Model select error:", e);
    } finally {
      setInstallingModel(null);
    }
  };

  const installSelectedModel = async () => {
    setInstallingModel(settings.asr.model);
    try {
      await api.installModel(settings.asr.model);
    } catch (error) {
      intelligence.notifyToast(
        "error",
        "Could not download the speech model. Check your connection and available disk space.",
      );
      console.error("Speech model install failed:", error);
    } finally {
      setInstallingModel(null);
    }
  };

  const handleSelectPrecision = async (id: Precision) => {
    if (settings.asr.precision === id) return;
    if (!(await change("asr", { ...settings.asr, precision: id }))) return;
    // The choice is already persisted to AppSettings, so a future launch
    // will honor it. Reload now so the new precision is live without
    // requiring a restart.
    if (modelStatus?.installed) {
      try {
        await api.reloadModel();
      } catch (e) {
        intelligence.notifyToast(
          "error",
          "Could not reload the speech model with this precision. Please try again.",
        );
        console.error("Precision reload error:", e);
      }
    }
  };

  const backendLabel = modelStatus?.backend ?? "";
  const modelReady = isModelReady(modelStatus);
  const downloading = Boolean(modelStatus?.is_downloading);
  const loading = isModelLoading(modelStatus);
  const speechBusy = installingModel !== null || downloading;

  // Parse the active mode out of the sidecar's backend string.
  // Format: "Qwen3-ASR 1.7B · CUDA int8" / "Qwen3-ASR 1.7B · CPU cpu"
  // Falls back to "—" when nothing useful is reported.
  const precisionLabel = (() => {
    if (!modelReady) return null;
    const m = backendLabel.match(/(CUDA|CPU)\s+(\S+)/i);
    if (!m) return null;
    const device = m[1].toUpperCase();
    const mode = m[2].toLowerCase();
    const vram =
      modelStatus && "vram_mb" in modelStatus
        ? (modelStatus as { vram_mb?: number }).vram_mb
        : undefined;
    const vramText = vram && vram > 0 ? ` · ${vram.toFixed(0)} MB VRAM` : "";
    return `Currently: ${device} ${mode}${vramText}`;
  })();

  const showGpuHint =
    Boolean(modelStatus?.gpu_available) &&
    modelStatus?.cuda_available === false &&
    Boolean(modelStatus?.asr_gpu_hint);

  /** Where a speech model's files land for the currently selected runtime. */
  const asrOnDiskId = (id: string) =>
    id === "phonon-2" || id === "zipformer-20m" || settings.asr.runtime !== "native"
      ? id
      : `native-${id}`;

  const asrEntry = (id: string) =>
    downloaded?.find((d) => d.kind === "asr" && d.id === asrOnDiskId(id)) ?? null;

  const approxDownloadSize = (id: string) =>
    DOWNLOAD_SIZE[id]?.[settings.asr.runtime === "native" ? "native" : "python"] ?? "";

  const gpuDetectionText = !capabilities
    ? (capabilitiesError ?? "Checking GPU support…")
    : refinementGpuAvailable
      ? `${detectedGpu?.name ?? "GPU"} · automatic full or partial offload`
      : "No compatible GPU detected";

  const confirmStrip = (key: string, prompt: React.ReactNode, onConfirm: () => void) => (
    <div className="mt-3 flex flex-wrap items-center justify-between gap-2 rounded-[var(--radius-control)] border border-danger/30 bg-danger-soft px-3 py-2">
      <p className="flex items-center gap-1.5 text-xs text-danger">
        <AlertTriangle className="w-3.5 h-3.5 shrink-0" {...ICON_PROPS} />
        {prompt}
      </p>
      <div className="flex items-center gap-2">
        <button
          type="button"
          className="btn btn-danger !py-1 !px-2.5 !text-xs"
          onClick={onConfirm}
          disabled={deleting !== null}
        >
          {deleting === key ? (
            <Loader2 className="w-3 h-3 animate-spin" aria-hidden />
          ) : (
            <Trash2 className="w-3 h-3" {...ICON_PROPS} />
          )}
          {deleting === key ? "Deleting…" : "Delete"}
        </button>
        <button
          type="button"
          className="btn btn-ghost !py-1 !px-2.5 !text-xs"
          onClick={() => setConfirmDelete(null)}
        >
          Cancel
        </button>
      </div>
    </div>
  );

  const statusDotClass = modelReady
    ? "bg-success"
    : downloading || loading
      ? "bg-accent"
      : modelStatus?.error
        ? "bg-danger"
        : "bg-faint/50";

  return (
    <>
      <Section
        icon={<Cpu className="w-4 h-4" {...ICON_PROPS} />}
        title="Speech model"
        description="Recognizes what you say, entirely on this computer."
      >
        <div>
          <div className="flex items-center justify-between gap-3">
            <div className="flex items-center gap-2.5 min-w-0">
              <span
                aria-hidden
                className={`w-2 h-2 mx-1 rounded-full shrink-0 ${statusDotClass}`}
              />
              <div className="min-w-0">
                <p className="text-sm font-medium text-ink truncate">
                  {modelStatus?.name || "Qwen3-ASR Engine"}
                </p>
                <p className="text-xs text-muted truncate">
                  {modelReady
                    ? modelStatus?.backend
                    : downloading
                      ? `Downloading weights · ${modelStatus?.download_progress_pct ?? 0}%`
                      : loading
                        ? `${
                            backendLabel && !/loading/i.test(backendLabel)
                              ? backendLabel
                              : `Loading ${phononSelected ? "Phonon-2" : zipformerSelected ? "Zipformer 20M" : settings.asr.model === "1.7b" ? "1.7B" : "0.6B"} model`
                          }`
                        : (modelStatus?.error ??
                          (modelStatus?.installed
                            ? "Model is not loaded"
                            : "Speech model is not installed"))}
                </p>
                {precisionLabel && (
                  <p className="text-2xs text-accent mt-0.5 font-medium">{precisionLabel}</p>
                )}
              </div>
            </div>
            {modelStatus?.installed && (
              <button
                type="button"
                className="icon-btn"
                title="Reload speech model"
                aria-label="Reload speech model"
                onClick={onReloadModel}
                disabled={loading || downloading || installingModel !== null}
              >
                <RefreshCw className="w-3.5 h-3.5" {...ICON_PROPS} />
              </button>
            )}
          </div>

          {downloading && (
            <div className="h-1.5 rounded-full bg-line mt-3 overflow-hidden">
              <div
                className="progress-fill bg-accent"
                style={{
                  transform: `scaleX(${Math.max(0, Math.min(100, modelStatus?.download_progress_pct ?? 0)) / 100})`,
                }}
              />
            </div>
          )}

          {modelStatus?.error && (
            <p className="text-xs text-danger mt-2 leading-relaxed font-medium">
              {modelStatus.error}
            </p>
          )}
          {modelStatus && !modelStatus.installed && !downloading && (
            <button
              type="button"
              className="btn btn-primary mt-3"
              onClick={installSelectedModel}
              disabled={installingModel !== null}
            >
              <Download className="w-4 h-4" {...ICON_PROPS} />
              {installingModel ? "Downloading…" : "Download speech model"}
            </button>
          )}
        </div>

        <ModelNotice notice={modelStatus?.asr_selection_notice} />

        {showGpuHint && (
          <div role="status" className="space-y-2">
            <div className="flex items-start gap-2 text-sm text-warning">
              <AlertTriangle className="w-4 h-4 mt-0.5 shrink-0" {...ICON_PROPS} />
              <div>
                <p className="font-medium">GPU detected, but speech is using CPU.</p>
                <p className="mt-1 leading-snug text-muted">
                  Your system Python&rsquo;s <code>torch</code> was installed without CUDA support.
                  Run the command below in a terminal, then reload the speech model:
                </p>
              </div>
            </div>
            <pre className="text-xs font-mono bg-surface border border-line rounded-[var(--radius-control)] px-2 py-1.5 overflow-x-auto whitespace-pre">
              {modelStatus?.asr_gpu_hint}
            </pre>
          </div>
        )}

        <ul aria-label="Speech models" className="divide-y divide-line-soft">
          {MODELS.map((m) => {
            const isActive = settings.asr.model === m.id;
            const entry = asrEntry(m.id);
            const partial = Boolean(entry?.partial);
            const installed = isActive
              ? Boolean(modelStatus?.installed)
              : Boolean(entry) && !partial;
            const hasFiles = Boolean(entry) || (isActive && Boolean(modelStatus?.installed));
            const key = `asr:${asrOnDiskId(m.id)}`;
            const sizeText = entry ? formatBytes(entry.size_bytes) : approxDownloadSize(m.id);
            const statusText =
              isActive && downloading
                ? `Downloading · ${modelStatus?.download_progress_pct ?? 0}%`
                : installingModel === m.id
                  ? "Downloading…"
                  : partial
                    ? "Incomplete download"
                    : installed
                      ? "Installed"
                      : "Not installed";
            return (
              <li key={m.id} className="py-3 first:pt-0 last:pb-0">
                <div className="flex items-center justify-between gap-4">
                  <div className="min-w-0">
                    <p className="text-sm font-medium text-ink">{m.title}</p>
                    <p className="text-xs text-muted leading-snug mt-0.5">{m.desc}</p>
                    <p className="text-2xs text-muted tabular-nums mt-1">
                      {entry ? sizeText : `${sizeText} download`} · {statusText}
                    </p>
                  </div>
                  <div className="flex items-center gap-1.5 shrink-0">
                    {isActive && installed ? (
                      <span className="inline-flex items-center gap-1 text-xs font-medium text-accent">
                        <Check className="w-3.5 h-3.5" {...ICON_PROPS} />
                        In use
                      </span>
                    ) : installed ? (
                      <button
                        type="button"
                        className="btn btn-secondary !py-1 !px-2.5 !text-xs"
                        aria-label={`Use ${m.title}`}
                        disabled={speechBusy || deleting !== null}
                        onClick={() => handleSelectModel(m.id)}
                      >
                        Use
                      </button>
                    ) : (
                      <button
                        type="button"
                        className="btn btn-primary !py-1 !px-2.5 !text-xs"
                        aria-label={
                          partial ? `Resume and use ${m.title}` : `Download and use ${m.title}`
                        }
                        disabled={speechBusy || deleting !== null}
                        onClick={() =>
                          isActive ? installSelectedModel() : handleSelectModel(m.id)
                        }
                      >
                        {installingModel === m.id && (
                          <Loader2 className="w-3 h-3 animate-spin" aria-hidden />
                        )}
                        {partial ? "Resume & use" : "Download & use"}
                      </button>
                    )}
                    {hasFiles && (
                      <button
                        type="button"
                        className="icon-btn"
                        aria-label={`Delete ${m.title}`}
                        title={`Delete ${m.title}`}
                        disabled={speechBusy || deleting !== null}
                        onClick={() => setConfirmDelete(key)}
                      >
                        <Trash2 className="w-3.5 h-3.5" {...ICON_PROPS} />
                      </button>
                    )}
                  </div>
                </div>
                {isActive && downloading && (
                  <div className="h-1.5 rounded-full bg-line mt-2 overflow-hidden">
                    <div
                      className="progress-fill bg-accent"
                      style={{
                        transform: `scaleX(${Math.max(0, Math.min(100, modelStatus?.download_progress_pct ?? 0)) / 100})`,
                      }}
                    />
                  </div>
                )}
                {confirmDelete === key &&
                  confirmStrip(
                    key,
                    <>
                      Delete {m.title} ({sizeText})?{" "}
                      {isActive
                        ? "This is the model you dictate with. Dictation stops until you download or choose another."
                        : "You can download it again later."}
                    </>,
                    () => deleteModel("asr", asrOnDiskId(m.id), m.title),
                  )}
              </li>
            );
          })}
        </ul>

        {phononSelected && (
          <p className="text-sm text-muted" role="status">
            English only. Choose English or Auto-detect for dictation. Download size is 164 MB;
            memory use is higher. CPU speed depends on your hardware. Dictionary replacements still
            apply; recognition hotword bias is unavailable.
          </p>
        )}
        {zipformerSelected && (
          <p className="text-sm text-muted" role="status">
            English only. Choose English or Auto-detect for dictation. Download size is 44 MB; runs
            on CPU with streaming partial results. Dictionary replacements still apply; recognition
            hotword bias is unavailable.
          </p>
        )}
      </Section>

      <Section
        icon={<Sparkles className="w-4 h-4" {...ICON_PROPS} />}
        title="Writing model"
        description="Optional. Rewrites your transcript locally after recognition."
      >
        <ul aria-label="Writing models" className="divide-y divide-line-soft">
          <li className="py-3 first:pt-0 last:pb-0">
            <div className="flex items-center justify-between gap-4">
              <div className="min-w-0">
                <p className="text-sm font-medium text-ink">No LLM · speech only</p>
                <p className="text-xs text-muted leading-snug mt-0.5">
                  Speech recognition and basic cleanup. Nothing to download.
                </p>
              </div>
              <button
                type="button"
                className="btn btn-secondary !py-1 !px-2.5 !text-xs shrink-0 aria-pressed:text-accent"
                aria-label="Use no LLM"
                aria-pressed={selectedLlm(settings) === "none"}
                disabled={selectingLlm}
                onClick={() => selectLlm("none")}
              >
                {selectedLlm(settings) === "none" ? "In use" : "Use"}
              </button>
            </div>
          </li>
          {(["smart_flow", "deep_context"] as IntelligenceTier[]).map((tier) => {
            const meta = INTELLIGENCE_TIERS[tier];
            const state = tierState(tier);
            const ggufInstalled = state?.weights_installed ?? state?.installed ?? false;
            const runtimeInstalled = flowStatus?.runtime_installed ?? false;
            const installed = ggufInstalled && runtimeInstalled;
            const showRuntimeMissing = ggufInstalled && !runtimeInstalled;
            const tierDownloading = isIntelligenceDownloading(tier);
            const isActive = selectedLlm(settings) === meta.modelId;
            const key = `llm:${tier}`;
            const statusText = tierDownloading
              ? `Downloading ${intelligenceDownload?.progress_pct ?? 0}% · ${(intelligenceDownload?.speed_mbps ?? 0).toFixed(1)} MB/s`
              : installed
                ? "Installed"
                : showRuntimeMissing
                  ? "Weights only"
                  : runtimeDownloadActive
                    ? "Installing runtime"
                    : "Not installed";
            return (
              <li key={tier} className="py-3 first:pt-0 last:pb-0">
                <div className="flex items-center justify-between gap-4">
                  <div className="min-w-0">
                    <p className="text-sm font-medium text-ink">{meta.label}</p>
                    <p className="text-xs text-muted leading-snug mt-0.5">{meta.tagline}</p>
                    <p className="text-2xs text-muted tabular-nums mt-1">
                      {FORMAT_SIZE(meta.downloadSizeMB)} download ·{" "}
                      {FORMAT_SIZE(meta.ramRequiredMB)} RAM · {statusText}
                    </p>
                  </div>
                  <div className="flex items-center gap-1.5 shrink-0">
                    <button
                      type="button"
                      className="btn btn-secondary !py-1 !px-2.5 !text-xs aria-pressed:text-accent"
                      aria-label={`Use ${meta.label}`}
                      aria-pressed={isActive}
                      disabled={selectingLlm}
                      onClick={() => selectLlm(meta.modelId)}
                    >
                      {isActive ? "In use" : "Use"}
                    </button>
                    {!installed &&
                      !showRuntimeMissing &&
                      !tierDownloading &&
                      !runtimeDownloadActive && (
                        <button
                          type="button"
                          className="btn btn-primary !py-1 !px-2.5 !text-xs"
                          onClick={() => handleInstallIntelligence(tier)}
                          disabled={installingIntelligence === tier}
                        >
                          {installingIntelligence === tier ? (
                            <Loader2 className="w-3 h-3 animate-spin" aria-hidden />
                          ) : (
                            <Download className="w-3 h-3" {...ICON_PROPS} />
                          )}
                          Download
                        </button>
                      )}
                    {showRuntimeMissing && (
                      <button
                        type="button"
                        className="btn btn-primary !py-1 !px-2.5 !text-xs"
                        onClick={onInstallRuntime}
                        disabled={runtimeDownloadActive}
                      >
                        {runtimeDownloadActive ? (
                          <Loader2 className="w-3 h-3 animate-spin" aria-hidden />
                        ) : (
                          <Cpu className="w-3 h-3" {...ICON_PROPS} />
                        )}
                        Install runtime
                      </button>
                    )}
                    {ggufInstalled && (
                      <button
                        type="button"
                        className="icon-btn"
                        aria-label={`Delete ${meta.label}`}
                        title={`Delete ${meta.label}`}
                        disabled={deleting !== null}
                        onClick={() => setConfirmDelete(key)}
                      >
                        <Trash2 className="w-3.5 h-3.5" {...ICON_PROPS} />
                      </button>
                    )}
                  </div>
                </div>

                {installed &&
                  flowStatus?.ready &&
                  flowStatus.active_model === meta.modelId &&
                  isActive && (
                    <p className="text-2xs text-muted leading-snug mt-2">
                      Loaded on {flowStatus.backend || "runtime"}
                      {flowStatus.n_gpu_layers !== undefined && (
                        <>
                          {" "}
                          · {flowStatus.n_gpu_layers}
                          {flowStatus.mode === "gpu" ? "/99 layers" : " layers"}
                          {flowStatus.mode === "gpu" && flowStatus.vram_used_mb
                            ? ` · ${flowStatus.vram_used_mb.toFixed(0)} MB VRAM`
                            : ""}
                        </>
                      )}
                    </p>
                  )}

                {showRuntimeMissing && (
                  <p className="text-2xs text-warning leading-snug mt-2">
                    This model is downloaded, but its writing runtime is missing. Install the
                    runtime to use it.
                  </p>
                )}
                {tierDownloading && (
                  <div className="space-y-1 mt-2">
                    <div className="flex items-center justify-between text-2xs text-muted">
                      <span>Downloading weights</span>
                      <span>
                        {intelligenceDownload?.progress_pct ?? 0}% ·{" "}
                        {(intelligenceDownload?.speed_mbps ?? 0).toFixed(1)} MB/s
                      </span>
                    </div>
                    <div className="h-1.5 rounded-full bg-line overflow-hidden">
                      <div
                        className="progress-fill bg-accent"
                        style={{
                          transform: `scaleX(${Math.max(0, Math.min(100, intelligenceDownload?.progress_pct ?? 0)) / 100})`,
                        }}
                      />
                    </div>
                  </div>
                )}
                {runtimeDownloadActive && runtimeDownload && (
                  <div className="space-y-1 mt-2">
                    <div className="flex items-center justify-between text-2xs text-muted">
                      <span>Installing {runtimeDownload.kind_label ?? "runtime"} runtime</span>
                      <span>
                        {runtimeDownload.progress_pct}% · {runtimeDownload.speed_mbps.toFixed(1)}{" "}
                        MB/s
                      </span>
                    </div>
                    <div className="h-1.5 rounded-full bg-line overflow-hidden">
                      <div
                        className="progress-fill bg-accent"
                        style={{
                          transform: `scaleX(${Math.max(0, Math.min(100, runtimeDownload.progress_pct)) / 100})`,
                        }}
                      />
                    </div>
                    {runtimeDownloadError && (
                      <p className="text-2xs text-danger leading-snug">{runtimeDownloadError}</p>
                    )}
                  </div>
                )}
                {showRuntimeMissing && !runtimeDownloadActive && runtimeDownloadError && (
                  <p className="text-2xs text-danger leading-snug mt-2">
                    Last install attempt failed: {runtimeDownloadError}
                  </p>
                )}
                {confirmDelete === key &&
                  confirmStrip(
                    key,
                    <>
                      Delete {meta.label} ({FORMAT_SIZE(meta.downloadSizeMB)})? You can download it
                      again later.
                    </>,
                    () => deleteModel("llm", tier, meta.label),
                  )}
              </li>
            );
          })}
        </ul>

        <div className="space-y-2">
          <Row
            label="Runs on"
            hint={`Choose CPU or GPU. GPU offload is tuned automatically for the lowest expected latency while reserving enough VRAM for speech recognition and the desktop. ${gpuDetectionText}`}
          >
            <div className="segmented-control" role="radiogroup" aria-label="Stage 2 processor">
              <button
                type="button"
                role="radio"
                aria-checked={refinementDevice === "cpu"}
                onClick={() => handleSelectRefinementDevice("cpu")}
              >
                CPU
              </button>
              <button
                type="button"
                role="radio"
                aria-checked={refinementDevice === "gpu"}
                disabled={!refinementGpuAvailable}
                onClick={() => handleSelectRefinementDevice("gpu")}
              >
                GPU
              </button>
            </div>
          </Row>
          {refinementDevice === "gpu" && (
            <p className="text-2xs text-muted leading-snug">
              {flowStatus?.ready && flowStatus.mode === "gpu"
                ? `${flowStatus.n_gpu_layers ?? 0} layers are currently offloaded to the GPU.`
                : flowStatus?.ready && flowStatus.mode === "cpu"
                  ? "The GPU runtime fell back to CPU. Reinstall the GPU runtime or free VRAM and try again."
                  : "The runtime will choose full offload when it fits, otherwise the fastest safe partial offload."}
            </p>
          )}
        </div>
      </Section>

      <Section
        icon={<SlidersHorizontal className="w-4 h-4" {...ICON_PROPS} />}
        title="Tuning"
        description="How Reflow balances speed, accuracy and memory."
      >
        <PresetSelector
          selectedPreset={settings.preset}
          capabilities={capabilities}
          onSelectPreset={(preset) => {
            void (async () => {
              if (!(await onUpdateSettings({ preset }))) return;
              try {
                await api.reloadModel();
                await intelligence.refresh();
              } catch (error) {
                intelligence.notifyToast("error", String(error));
              }
            })();
          }}
        />
        {settings.preset === "custom" ? (
          <>
            <Row
              label="Speech runtime"
              hint={
                phononSelected
                  ? "Phonon uses the Fermion Python runtime."
                  : zipformerSelected
                    ? "Zipformer uses the sherpa-onnx Python runtime."
                    : "Native is an experimental Python-free runtime; download its model files separately."
              }
            >
              <select
                className="field"
                value={settings.asr.runtime}
                disabled={pythonOnlySelected || modelStatus?.is_downloading}
                onChange={async (e) => {
                  const runtime = e.target.value as AsrSettings["runtime"];
                  if (!(await change("asr", { ...settings.asr, runtime }))) return;
                  try {
                    const status = await api.getModelStatus();
                    if (status.installed) await api.reloadModel();
                  } catch (error) {
                    intelligence.notifyToast(
                      "error",
                      "Could not switch speech runtime. Check the model and runtime downloads.",
                    );
                    console.error("Runtime switch error:", error);
                  }
                }}
              >
                <option value="python">Python (default)</option>
                <option value="native">Native (experimental)</option>
              </select>
            </Row>

            <Row
              label="Compute backend"
              hint={
                settings.asr.runtime === "native"
                  ? "Native uses Vulkan when a compatible GPU is present"
                  : "Auto uses CUDA when a compatible GPU is present"
              }
            >
              <select
                className="field"
                value={settings.asr.device}
                onChange={(e) =>
                  change("asr", {
                    ...settings.asr,
                    device: e.target.value as AsrSettings["device"],
                  })
                }
              >
                <option value="auto">Auto (GPU prioritized)</option>
                <option value="cuda">
                  GPU only ({settings.asr.runtime === "native" ? "Vulkan" : "CUDA"})
                </option>
                <option value="cpu">CPU only</option>
              </select>
            </Row>

            {settings.asr.runtime === "python" && !pythonOnlySelected && (
              <Row
                label="Model precision"
                hint="Lower precision uses less VRAM; 16-bit is the most accurate. The choice is remembered across launches."
              >
                <div className="flex flex-col items-end gap-1">
                  <div className="segmented-control">
                    {PRECISIONS.map((p) => (
                      <button
                        key={p.id}
                        type="button"
                        aria-pressed={settings.asr.precision === p.id}
                        onClick={() => handleSelectPrecision(p.id)}
                        disabled={downloading || installingModel !== null}
                      >
                        {p.title}
                      </button>
                    ))}
                  </div>
                  <p className="text-2xs text-muted text-right leading-snug max-w-56">
                    {PRECISIONS.find((p) => p.id === settings.asr.precision)?.desc}
                  </p>
                </div>
              </Row>
            )}

            <Row label="Keep model loaded" hint="Pre-warms the model in memory at startup">
              <Toggle
                on={settings.asr.keep_loaded}
                onChange={(v) => change("asr", { ...settings.asr, keep_loaded: v })}
                ariaLabel="Keep model loaded"
              />
            </Row>
          </>
        ) : (
          <p className="text-sm text-muted">
            Speech runtime, precision and compute settings are managed automatically. Select Custom
            above to tune them manually.
          </p>
        )}
      </Section>

      <Section
        icon={<HardDrive className="w-4 h-4" {...ICON_PROPS} />}
        title="On this computer"
        description={
          downloaded === null
            ? "Checking downloads…"
            : downloaded.length === 0
              ? "Nothing downloaded yet."
              : `${formatBytes(downloaded.reduce((sum, m) => sum + m.size_bytes, 0))} of models and runtimes.`
        }
      >
        {downloaded === null || downloaded.length === 0 ? (
          <p className="text-sm text-muted">Models and runtimes you download appear here.</p>
        ) : (
          <ul aria-label="Downloaded files" className="divide-y divide-line-soft">
            {downloaded.map((m) => {
              const key = `${m.kind}:${m.id}`;
              const confirmKey = `disk:${key}`;
              return (
                <li key={key} className="py-3 first:pt-0 last:pb-0">
                  <div className="flex items-center justify-between gap-4">
                    <div className="min-w-0">
                      <p className="text-sm font-medium text-ink truncate">{m.label}</p>
                      <p className="text-2xs text-muted tabular-nums">
                        {DOWNLOADED_KIND_LABEL[m.kind]} · {formatBytes(m.size_bytes)}
                        {m.partial ? " · incomplete download" : ""}
                      </p>
                    </div>
                    <div className="flex items-center gap-2 shrink-0">
                      {m.active && <span className="chip">In use</span>}
                      <button
                        type="button"
                        className="icon-btn"
                        aria-label={`Delete ${m.label}`}
                        title={`Delete ${m.label}`}
                        disabled={deleting !== null}
                        onClick={() => setConfirmDelete(confirmKey)}
                      >
                        <Trash2 className="w-3.5 h-3.5" {...ICON_PROPS} />
                      </button>
                    </div>
                  </div>
                  {confirmDelete === confirmKey &&
                    confirmStrip(
                      key,
                      <>
                        Delete {m.label} ({formatBytes(m.size_bytes)})?{" "}
                        {m.active
                          ? m.kind === "asr"
                            ? "This is the model you dictate with. Dictation stops until you download or choose another."
                            : "This model is currently in use."
                          : "You can download it again later."}
                      </>,
                      () => deleteModel(m.kind, m.id, m.label),
                    )}
                </li>
              );
            })}
          </ul>
        )}
      </Section>

      <Section icon={<Wrench className="w-4 h-4" {...ICON_PROPS} />} title="Diagnostics">
        <RuntimeInventoryPanel busy={runtimeDownloadActive} />
        <CalibrationPanel />
      </Section>
    </>
  );
};
