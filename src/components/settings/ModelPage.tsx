import React, { useState } from "react";
import {
  AppSettings,
  AsrSettings,
  Capabilities,
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
import type { IntelligenceHub } from "../../hooks/useIntelligenceHub";

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

const MODELS: { id: "0.6b" | "1.7b"; title: string; desc: string }[] = [
  { id: "0.6b", title: "0.6B · Faster", desc: "Smaller model · speed depends on your hardware" },
  { id: "1.7b", title: "1.7B · Higher accuracy", desc: "Larger model · requires more memory" },
];

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
  // Accepted but not wired yet; Task 39 adds the Remove action to this page.
  onRemoveRuntime: _onRemoveRuntime,
}) => {
  const [installingModel, setInstallingModel] = useState<string | null>(null);
  const [removing, setRemoving] = useState(false);
  const [confirmRemove, setConfirmRemove] = useState(false);
  const { flowStatus, intelligenceTiers } = intelligence;
  const [capabilities, setCapabilities] = useState<Capabilities | null>(null);
  const [capabilitiesError, setCapabilitiesError] = useState<string | null>(null);

  const [installingIntelligence, setInstallingIntelligence] = useState<IntelligenceTier | null>(
    null,
  );
  const [removingIntelligence, setRemovingIntelligence] = useState<IntelligenceTier | null>(null);
  const [confirmRemoveIntelligence, setConfirmRemoveIntelligence] =
    useState<IntelligenceTier | null>(null);

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
        "The model operation could not be completed. Please try again or check the logs.",
      );
      console.error("installIntelligenceModel failed", e);
    } finally {
      setInstallingIntelligence(null);
    }
  };

  const handleRemoveIntelligence = async (tier: IntelligenceTier) => {
    if (tier === "raw_verbatim") return;
    setRemovingIntelligence(tier);
    try {
      await api.removeIntelligenceModel(tier);
    } catch (e) {
      intelligence.notifyToast(
        "error",
        "The model operation could not be completed. Please try again or check the logs.",
      );
      console.error("removeIntelligenceModel failed", e);
    } finally {
      setRemovingIntelligence(null);
      setConfirmRemoveIntelligence(null);
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

  const handleSelectModel = async (id: "0.6b" | "1.7b") => {
    if (settings.asr.model === id) return;
    if (!(await change("asr", { ...settings.asr, model: id }))) return;
    try {
      const status = await api.getModelStatus();
      if (!status.installed) {
        setInstallingModel(id);
        await api.installModel(id);
      } else {
        await api.reloadModel();
      }
    } catch (e) {
      intelligence.notifyToast(
        "error",
        "The model operation could not be completed. Please try again or check the logs.",
      );
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

  const removeModel = async () => {
    setRemoving(true);
    try {
      await api.removeModel(settings.asr.model);
    } catch (e) {
      intelligence.notifyToast(
        "error",
        "The model operation could not be completed. Please try again or check the logs.",
      );
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

  return (
    <Section icon={<Cpu className="w-4 h-4" />} title="Speech model">
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
      {/* Why the running model may not be the one selected below. */}
      <ModelNotice notice={modelStatus?.asr_selection_notice} />
      {showGpuHint && (
        <div
          className="rounded-xl border border-warning/40 bg-warning/10 p-3 space-y-1.5"
          role="status"
        >
          <div className="flex items-start gap-2 text-sm text-warning">
            <AlertTriangle className="w-4 h-4 mt-0.5 shrink-0" />
            <div>
              <p className="font-semibold">GPU detected, but ASR is using CPU.</p>
              <p className="mt-1 leading-snug">
                Your system Python&rsquo;s <code>torch</code> was installed without CUDA support.
                Run the command below in a terminal, then click <em>Reload speech model</em>:
              </p>
            </div>
          </div>
          <pre className="text-xs font-mono bg-surface-2 border border-warning/30 rounded-md px-2 py-1.5 overflow-x-auto whitespace-pre">
            {modelStatus?.asr_gpu_hint}
          </pre>
        </div>
      )}
      <div className="grid grid-cols-2 gap-3">
        {MODELS.map((m) => {
          const active = settings.asr.model === m.id;
          return (
            <button
              key={m.id}
              onClick={() => handleSelectModel(m.id)}
              disabled={installingModel !== null || downloading}
              aria-pressed={active}
              className={`text-left rounded-xl border p-3.5 transition-all cursor-pointer ${
                active
                  ? "border-accent bg-accent-soft shadow-xs ring-1 ring-accent"
                  : "border-line bg-surface hover:border-line-strong hover:bg-base-2"
              }`}
            >
              <div className="flex items-center justify-between">
                <p className={`text-sm font-semibold ${active ? "text-accent" : "text-ink"}`}>
                  {m.title}
                </p>
                {active && (
                  <div className="w-4 h-4 rounded-full bg-accent text-white flex items-center justify-center">
                    <Check className="w-2.5 h-2.5 stroke-[3]" />
                  </div>
                )}
              </div>
              <p className="text-xs text-muted mt-1 leading-snug">{m.desc}</p>
              {installingModel === m.id && (
                <p className="text-2xs text-accent mt-2 flex items-center gap-1.5 font-medium">
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
                ? "border-danger/30 bg-danger/10"
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
              <p className="text-sm font-semibold text-ink truncate">
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
                            : `Loading ${settings.asr.model === "1.7b" ? "1.7B" : "0.6B"} model`
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
              className="icon-btn hover:bg-surface-2 hover:text-accent shadow-xs border border-line"
              title="Reload speech model"
              aria-label="Reload speech model"
              onClick={onReloadModel}
              disabled={loading || downloading || installingModel !== null}
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
          <p className="text-xs text-danger mt-2 leading-relaxed font-medium">
            {modelStatus.error}
          </p>
        )}
        {modelStatus && !modelStatus.installed && !downloading && (
          <button
            className="btn btn-primary mt-3"
            onClick={installSelectedModel}
            disabled={installingModel !== null}
          >
            <Download className="w-4 h-4" />
            {installingModel ? "Downloading…" : "Download speech model"}
          </button>
        )}
      </div>

      {settings.preset === "custom" ? (
        <>
          <Row
            label="Speech runtime"
            hint="Native is an experimental Python-free runtime; download its model files separately."
          >
            <select
              className="field"
              value={settings.asr.runtime}
              disabled={modelStatus?.is_downloading}
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
                change("asr", { ...settings.asr, device: e.target.value as AsrSettings["device"] })
              }
            >
              <option value="auto">Auto (GPU prioritized)</option>
              <option value="cuda">
                GPU only ({settings.asr.runtime === "native" ? "Vulkan" : "CUDA"})
              </option>
              <option value="cpu">CPU only</option>
            </select>
          </Row>

          {settings.asr.runtime === "python" && (
            <div>
              <div className="flex items-center justify-between gap-6 mb-1.5">
                <div className="min-w-0">
                  <p className="text-sm text-ink font-medium">Model precision</p>
                  <p className="text-xs text-muted mt-0.5 leading-relaxed">
                    Lower precision uses less VRAM; 16-bit is the most accurate. The choice is
                    remembered across launches.
                  </p>
                </div>
              </div>
              <div className="grid grid-cols-4 gap-2">
                {PRECISIONS.map((p) => {
                  const active = settings.asr.precision === p.id;
                  return (
                    <button
                      key={p.id}
                      onClick={() => handleSelectPrecision(p.id)}
                      disabled={downloading || installingModel !== null}
                      aria-pressed={active}
                      className={`text-left rounded-lg border p-2.5 transition-all cursor-pointer ${
                        active
                          ? "border-accent bg-accent-soft shadow-xs ring-1 ring-accent"
                          : "border-line bg-surface hover:border-line-strong hover:bg-base-2"
                      }`}
                    >
                      <div className="flex items-center justify-between">
                        <p
                          className={`text-sm font-semibold ${active ? "text-accent" : "text-ink"}`}
                        >
                          {p.title}
                        </p>
                        {active && (
                          <div className="w-3.5 h-3.5 rounded-full bg-accent text-white flex items-center justify-center">
                            <Check className="w-2 h-2 stroke-[3]" />
                          </div>
                        )}
                      </div>
                      <p className="text-2xs text-muted mt-0.5 leading-snug">{p.desc}</p>
                    </button>
                  );
                })}
              </div>
            </div>
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

      {modelStatus?.installed && !confirmRemove && (
        <button className="btn btn-danger w-full mt-2" onClick={() => setConfirmRemove(true)}>
          <Trash2 className="w-3.5 h-3.5" />
          Remove downloaded model weights
        </button>
      )}
      {modelStatus?.installed && confirmRemove && (
        <div className="p-3 rounded-xl border border-danger/30 bg-danger/10 space-y-2">
          <div className="flex items-start gap-2 text-sm text-danger">
            <AlertTriangle className="w-4 h-4 mt-0.5 shrink-0" />
            <p>
              This deletes the {settings.asr.model.toUpperCase()} weights from your computer. You'll
              need to re-download to dictate again.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <button
              className="btn btn-danger !py-1.5 !px-3 !text-sm"
              onClick={removeModel}
              disabled={removing}
            >
              <Trash2 className="w-3.5 h-3.5" />
              {removing ? "Removing…" : "Yes, remove"}
            </button>
            <button
              className="btn btn-ghost !py-1.5 !px-3 !text-sm"
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
          <h3 className="text-sm font-semibold text-ink">Intelligence &amp; Flow Engine</h3>
        </div>
        <p className="text-xs text-muted -mt-1">
          Stage 2 LLM post-processing. Download the GGUF weights for the tier you want to enable.
        </p>

        {(["smart_flow", "deep_context"] as IntelligenceTier[]).map((tier) => {
          const meta = INTELLIGENCE_TIERS[tier];
          const state = tierState(tier);
          const ggufInstalled = state?.weights_installed ?? state?.installed ?? false;
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
                isActive ? "border-accent bg-accent-soft" : "border-line bg-surface"
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
                    <p className="text-sm font-semibold text-ink">{meta.label}</p>
                    {isActive && (
                      <span className="text-2xs font-bold tracking-wider px-1.5 py-0.5 rounded bg-accent text-white">
                        ACTIVE
                      </span>
                    )}
                  </div>
                  <p className="text-xs text-muted mt-0.5 break-all">{meta.modelFile}</p>
                </div>
              </div>

              <div className="flex items-center justify-between gap-3 text-xs text-muted">
                <span>
                  <span className="text-ink font-medium">{FORMAT_SIZE(meta.downloadSizeMB)}</span>{" "}
                  download ·{" "}
                  <span className="text-ink font-medium">{FORMAT_SIZE(meta.ramRequiredMB)}</span>{" "}
                  RAM
                </span>
                {installed && (
                  <span className="inline-flex items-center gap-1 text-success font-semibold">
                    <ShieldCheck className="w-3.5 h-3.5" />
                    Installed
                  </span>
                )}
                {showRuntimeMissing && (
                  <span className="inline-flex items-center gap-1 text-warning font-semibold">
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
                <div className="text-2xs text-muted bg-base-2/60 border border-line rounded-md px-2.5 py-1.5 leading-snug">
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
                </div>
              )}

              {showRuntimeMissing && (
                <p className="text-2xs text-warning leading-snug">
                  Weights are downloaded, but the <code>llama-server</code> runtime is missing from{" "}
                  <code>~/AppData/Roaming/reflow/bin/</code>. The tier can&rsquo;t run until the
                  runtime is installed.
                </p>
              )}
              {downloading && (
                <div className="space-y-1">
                  <div className="flex items-center justify-between text-2xs text-muted">
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
                  <div className="flex items-center justify-between text-2xs text-muted">
                    <span>Installing {runtimeDownload.kind_label ?? "runtime"} runtime</span>
                    <span>
                      {runtimeDownload.progress_pct}% · {runtimeDownload.speed_mbps.toFixed(1)} MB/s
                    </span>
                  </div>
                  <div className="h-1.5 rounded-full bg-line overflow-hidden">
                    <div
                      className="h-full bg-accent transition-all duration-300 rounded-full"
                      style={{ width: `${runtimeDownload.progress_pct}%` }}
                    />
                  </div>
                  {runtimeDownloadError && (
                    <p className="text-2xs text-danger leading-snug">{runtimeDownloadError}</p>
                  )}
                </div>
              )}
              {showRuntimeMissing && !runtimeDownloadActive && runtimeDownloadError && (
                <p className="text-2xs text-danger leading-snug">
                  Last install attempt failed: {runtimeDownloadError}
                </p>
              )}
              {confirmRemoveIntelligence === tier && (
                <div className="p-2.5 rounded-lg border border-danger/30 bg-danger/10 space-y-1.5">
                  <p className="text-xs text-danger">
                    Delete the downloaded weights for {meta.label}? You can re-download later.
                  </p>
                  <div className="flex items-center gap-2">
                    <button
                      className="btn btn-danger !py-1 !px-2.5 !text-xs"
                      onClick={() => handleRemoveIntelligence(tier)}
                      disabled={isRemoving}
                    >
                      <Trash2 className="w-3 h-3" />
                      {isRemoving ? "Removing…" : "Yes, remove"}
                    </button>
                    <button
                      className="btn btn-ghost !py-1 !px-2.5 !text-xs"
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
                    className="btn btn-ghost !py-1 !px-2.5 !text-xs"
                    onClick={() => setConfirmRemoveIntelligence(tier)}
                    disabled={isRemoving}
                  >
                    <Trash2 className="w-3 h-3" />
                    Remove
                  </button>
                ) : showRuntimeMissing ? (
                  <>
                    <button
                      className="btn btn-primary !py-1 !px-2.5 !text-xs"
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
                      className="btn btn-ghost !py-1 !px-2.5 !text-xs"
                      onClick={() => setConfirmRemoveIntelligence(tier)}
                      disabled={isRemoving}
                    >
                      <Trash2 className="w-3 h-3" />
                      Remove
                    </button>
                  </>
                ) : downloading || runtimeDownloadActive ? null : (
                  <button
                    className="btn btn-primary !py-1 !px-2.5 !text-xs"
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
            <p className="text-sm text-ink font-medium">Stage 2 processor</p>
          </div>
          <p className="text-xs text-muted leading-snug">
            Choose CPU or GPU. GPU offload is tuned automatically for the lowest expected latency
            while reserving enough VRAM for speech recognition and the desktop.
          </p>
          <div className="grid grid-cols-2 gap-2" role="radiogroup" aria-label="Stage 2 processor">
            <button
              type="button"
              role="radio"
              aria-checked={refinementDevice === "cpu"}
              onClick={() => handleSelectRefinementDevice("cpu")}
              className={`text-left rounded-lg border p-3 transition-all cursor-pointer ${
                refinementDevice === "cpu"
                  ? "border-accent bg-accent-soft shadow-xs ring-1 ring-accent"
                  : "border-line bg-surface hover:border-line-strong hover:bg-base-2"
              }`}
            >
              <div className="flex items-center justify-between gap-3">
                <div className="flex items-center gap-2">
                  <Cpu className="w-4 h-4 text-muted" />
                  <p className="text-sm font-semibold text-ink">CPU</p>
                </div>
                {refinementDevice === "cpu" && (
                  <div className="w-4 h-4 rounded-full bg-accent text-white flex items-center justify-center">
                    <Check className="w-2.5 h-2.5 stroke-[3]" />
                  </div>
                )}
              </div>
              <p className="text-2xs text-muted mt-1 leading-snug">
                Keeps every refinement layer in system memory.
              </p>
            </button>

            <button
              type="button"
              role="radio"
              aria-checked={refinementDevice === "gpu"}
              aria-disabled={!refinementGpuAvailable}
              disabled={!refinementGpuAvailable}
              onClick={() => handleSelectRefinementDevice("gpu")}
              className={`text-left rounded-lg border p-3 transition-all ${
                refinementDevice === "gpu"
                  ? "border-accent bg-accent-soft shadow-xs ring-1 ring-accent"
                  : "border-line bg-surface"
              } ${
                refinementGpuAvailable
                  ? "cursor-pointer hover:border-line-strong hover:bg-base-2"
                  : "cursor-not-allowed opacity-55"
              }`}
            >
              <div className="flex items-center justify-between gap-3">
                <div className="flex items-center gap-2">
                  <Zap className="w-4 h-4 text-muted" />
                  <p className="text-sm font-semibold text-ink">GPU</p>
                </div>
                {refinementDevice === "gpu" && (
                  <div className="w-4 h-4 rounded-full bg-accent text-white flex items-center justify-center">
                    <Check className="w-2.5 h-2.5 stroke-[3]" />
                  </div>
                )}
              </div>
              <p className="text-2xs text-muted mt-1 leading-snug">
                {!capabilities
                  ? (capabilitiesError ?? "Checking GPU support…")
                  : refinementGpuAvailable
                    ? `${detectedGpu?.name ?? "GPU"} · automatic full or partial offload`
                    : "No compatible GPU detected"}
              </p>
            </button>
          </div>
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
      </div>
      <RuntimeInventoryPanel busy={runtimeDownloadActive} />
      <CalibrationPanel />
    </Section>
  );
};
