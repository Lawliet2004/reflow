import React, { useState } from "react";
import {
  AppSettings,
  IntelligenceTier,
  WritingModelDownloadId,
  INTELLIGENCE_TIERS,
  ModelStatus,
  RuntimeDownloadEvent,
  TranscriptStyle,
  TranscriptFormat,
  tierState,
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
import { WritingTaskPicker } from "../WritingTaskPicker";
import { LlmSelector } from "../LlmSelector";
import { flowModelForTier } from "../../types";
import { llmSelectionPatch } from "../../llmSelection";

interface Props {
  intelligence: IntelligenceHub;
  settings: AppSettings;
  onUpdateSettings: (s: Partial<AppSettings>) => Promise<boolean>;
  modelStatus: ModelStatus | null;
  intelligenceDownload: IntelligenceDownloadEvent | null;
  activeDownloadTiers: Set<WritingModelDownloadId>;
  runtimeDownload: RuntimeDownloadEvent | null;
  runtimeDownloadActive: boolean;
  runtimeDownloadError: string | null;
  onInstallRuntime: () => void;
}

const STYLE_OPTIONS: { value: TranscriptStyle; label: string }[] = [
  { value: "faithful", label: "Faithful" },
  { value: "neutral", label: "Neutral" },
  { value: "decisive", label: "Decisive" },
  { value: "email", label: "Email" },
  { value: "chat", label: "Chat" },
];

const TRANSCRIPT_FORMATS: { value: TranscriptFormat; label: string }[] = [
  { value: "off", label: "Off / Existing behavior" },
  { value: "auto", label: "Auto · recommended" },
  { value: "natural", label: "Natural" },
  { value: "technical", label: "Technical" },
  { value: "brainstorm", label: "Brainstorm" },
  { value: "meeting", label: "Meeting Notes" },
  { value: "steps", label: "Steps" },
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
}) => {
  const { flowStatus, intelligenceTiers, systemMetrics } = intelligence;

  const [installing, setInstalling] = useState<IntelligenceTier | null>(null);
  const [sample, setSample] = useState("I want to drink coffee um no wait I want tea");
  const [previewOut, setPreviewOut] = useState("");
  const [previewLatency, setPreviewLatency] = useState<number | null>(null);
  const [previewModel, setPreviewModel] = useState<string>("");
  const [previewUsed, setPreviewUsed] = useState(false);
  const [previewNotice, setPreviewNotice] = useState<string | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [removing, setRemoving] = useState<IntelligenceTier | null>(null);
  const [formatDraft, setFormatDraft] = useState<{ source: string; value: string } | null>(null);
  const formatSource = settings.custom_format_instructions ?? "";
  const formatInstructionDraft =
    formatDraft?.source === formatSource ? formatDraft.value : formatSource;
  const setFormatInstructionDraft = (value: string) =>
    setFormatDraft({ source: formatSource, value });
  const [formatSaving, setFormatSaving] = useState(false);
  const [formatError, setFormatError] = useState<string | null>(null);

  const tier: IntelligenceTier = settings.intelligence_tier ?? "smart_flow";
  const transcriptFormat = settings.transcript_format ?? "off";
  const formattingEnabled = transcriptFormat !== "off";

  const hasGpu = Boolean(systemMetrics?.gpu_present || modelStatus?.gpu_available);
  const totalRamMb = systemMetrics?.total_ram_mb ?? 0;
  const lowSpecPc = !hasGpu || totalRamMb < 8 * 1024;

  const isInstalled = (t: IntelligenceTier) => tierState(intelligenceTiers, t)?.installed ?? false;

  // The runtime is a single binary that gates every non-raw tier. We
  // know a tier is "weights only" when its GGUF is on disk (per the
  // per-tier state, which the Rust side keeps truthful) and the
  // runtime is not.
  const runtimeInstalled = flowStatus?.runtime_installed ?? false;
  const isWeightsOnly = (t: IntelligenceTier) => {
    if (t === "raw_verbatim") return false;
    const state = tierState(intelligenceTiers, t);
    const ggufInstalled = state?.weights_installed ?? state?.installed ?? false;
    if (!ggufInstalled) return false;
    return !runtimeInstalled;
  };

  const isDownloading = (t: IntelligenceTier) => activeDownloadTiers.has(t);

  const selectedMode = settings.modes?.find(
    (mode) => mode.id === settings.default_mode_id && mode.enabled,
  );
  const previewMode = selectedMode?.id !== "dictation" ? selectedMode : undefined;
  const formattingTier = previewMode?.intelligence_tier ?? tier;
  const formattingUnavailable = formattingEnabled
    ? previewMode?.dictation_mode === "coding"
      ? "Literal Coding mode keeps code unchanged and skips AI formatting."
      : previewMode &&
          (previewMode.intelligence_tier === "raw_verbatim" || previewMode.cleanup_level === "raw")
        ? `The ${previewMode.name} mode is set to No LLM or Raw cleanup, so intelligent formatting is paused.`
        : formattingTier === "raw_verbatim"
          ? "Select Qwen3.5 0.8B or Qwen3.5 2B in Cleanup models to enable AI formatting. Reflow will not change your model selection automatically."
          : isWeightsOnly(formattingTier)
            ? "The selected model weights are present, but the llama.cpp runtime is missing. Install the runtime in Cleanup models to use AI formatting."
            : !isInstalled(formattingTier)
              ? "The selected local model is not installed. Formatting will use deterministic cleanup until it is available."
              : null
    : null;

  const updateTranscriptFormat = async (value: TranscriptFormat) => {
    setFormatError(null);
    try {
      if (!(await onUpdateSettings({ transcript_format: value }))) {
        setFormatError("Could not save the formatting preference. Try again.");
      }
    } catch {
      setFormatError("Could not save the formatting preference. Try again.");
    }
  };

  const saveCustomFormatInstruction = async (value: string) => {
    setFormatSaving(true);
    setFormatError(null);
    try {
      if (!(await onUpdateSettings({ custom_format_instructions: value.slice(0, 500) }))) {
        setFormatError("Could not save the custom instruction. Try again.");
      }
    } catch {
      setFormatError("Could not save the custom instruction. Try again.");
    } finally {
      setFormatSaving(false);
    }
  };

  const eventForTier = (t: IntelligenceTier) =>
    intelligence.intelligenceDownloads?.[t] ??
    (intelligenceDownload?.tier === t ? intelligenceDownload : null);
  const downloadProgress = (t: IntelligenceTier) =>
    isDownloading(t) ? (eventForTier(t)?.progress_pct ?? 0) : 0;
  const downloadSpeed = (t: IntelligenceTier) =>
    isDownloading(t) ? (eventForTier(t)?.speed_mbps ?? 0) : 0;

  const handleSelectTier = (t: IntelligenceTier) => {
    const selected =
      settings.flow_model === "none" ? settings.last_polish_model : settings.flow_model;
    const model =
      t !== "raw_verbatim" &&
      selected &&
      ["qwen3.5-4b", "minicpm5-2b", "gemma4-e4b"].includes(selected)
        ? selected
        : flowModelForTier(t);
    onUpdateSettings({ ...llmSelectionPatch(settings, model), intelligence_tier: t });
  };

  const handleInstall = async (t: IntelligenceTier) => {
    if (t === "raw_verbatim") return;
    if (activeDownloadTiers.has(t)) return;
    if (tierState(intelligenceTiers, t)?.installed) return;
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
    setPreviewOut("");
    setPreviewLatency(null);
    setPreviewNotice(null);
    try {
      const result = await api.previewTierCleanup(sample, tier, settings.style, previewMode);
      setPreviewOut(result.text);
      setPreviewLatency(result.latency_ms);
      setPreviewModel(result.model_used);
      setPreviewUsed(result.rewriter_used === true);
      setPreviewNotice(result.rewriter_error ?? null);
    } catch (e) {
      intelligence.notifyToast(
        "error",
        "The model operation could not be completed. Please try again or check the logs.",
      );
      console.error("preview failed", e);
      setPreviewOut("");
      setPreviewLatency(null);
      setPreviewModel("");
      setPreviewUsed(false);
    } finally {
      setPreviewing(false);
    }
  };

  const activeMeta = INTELLIGENCE_TIERS[tier];

  return (
    <Section
      icon={<Sparkles className="w-4 h-4 text-accent" />}
      title="Writing & cleanup"
      description="Choose the task, then tune the edit. Speech recognition and text cleanup are separate choices."
    >
      <WritingTaskPicker settings={settings} onUpdateSettings={onUpdateSettings} />
      <LlmSelector
        settings={settings}
        onUpdateSettings={onUpdateSettings}
        tiers={intelligenceTiers}
      />
      <p className="text-xs text-muted leading-relaxed">
        Cleanup keeps your meaning, numbers and technical identifiers. If the model is unavailable
        or its edit is rejected, Reflow keeps the rule-cleaned transcript and reports the reason.
      </p>
      <div className="rounded-xl border border-line bg-surface p-3.5 space-y-3">
        <div className="space-y-1">
          <label htmlFor="transcript-format" className="text-sm font-semibold text-ink">
            Intelligent Formatting
          </label>
          <p id="transcript-format-help" className="text-xs text-muted leading-relaxed">
            Uses one pass with the selected local LLM. Off keeps your current cleanup behavior.
          </p>
        </div>
        <select
          id="transcript-format"
          className="field w-full"
          value={transcriptFormat}
          aria-describedby="transcript-format-help"
          onChange={(event) => {
            void updateTranscriptFormat(event.target.value as TranscriptFormat);
          }}
        >
          {TRANSCRIPT_FORMATS.map((format) => (
            <option key={format.value} value={format.value}>
              {format.label}
            </option>
          ))}
        </select>
        {formattingUnavailable && (
          <p role="status" className="text-xs text-warning leading-relaxed">
            {formattingUnavailable}
          </p>
        )}
        <details>
          <summary className="text-sm text-ink cursor-pointer">
            Custom formatting instruction
          </summary>
          <div className="space-y-2.5 pt-2">
            <textarea
              aria-label="Custom formatting instruction"
              className="field w-full min-h-[72px] resize-y"
              value={formatInstructionDraft}
              maxLength={500}
              onChange={(event) => setFormatInstructionDraft(event.target.value)}
              placeholder="Add a short preference for structure or presentation. Factual safeguards always apply."
            />
            <div className="flex items-center justify-between gap-2">
              <span className="text-2xs text-muted">
                {Array.from(formatInstructionDraft).length}/500 characters
              </span>
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  className="btn btn-ghost !py-1.5 !px-2.5 !text-xs"
                  disabled={formatSaving || !settings.custom_format_instructions}
                  onClick={() => {
                    setFormatInstructionDraft("");
                    void saveCustomFormatInstruction("");
                  }}
                >
                  Restore default
                </button>
                <button
                  type="button"
                  className="btn btn-secondary !py-1.5 !px-2.5 !text-xs"
                  disabled={formatSaving}
                  onClick={() => {
                    void saveCustomFormatInstruction(formatInstructionDraft);
                  }}
                >
                  {formatSaving ? "Saving…" : "Save instruction"}
                </button>
              </div>
            </div>
            <p className="text-2xs text-muted leading-relaxed">
              Changes presentation only. It cannot change facts, follow dictated instructions or
              affect Raw/Verbatim and literal Coding modes.
            </p>
          </div>
        </details>
        {formatError && (
          <p role="alert" className="text-xs text-danger leading-relaxed">
            {formatError}
          </p>
        )}
      </div>
      {runtimeDownloadError && (
        <p role="alert" className="text-sm text-danger">
          Runtime installation failed: {runtimeDownloadError}
        </p>
      )}
      <details>
        <summary className="text-sm font-medium text-ink cursor-pointer mb-3">
          Cleanup models &amp; downloads
        </summary>
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
                            {eventForTier(id)?.phase === "checking_runtime"
                              ? "Checking writing runtime…"
                              : eventForTier(id)?.phase === "verifying"
                                ? "Verifying model…"
                                : eventForTier(id)?.phase === "committing"
                                  ? "Finishing setup…"
                                  : eventForTier(id)?.phase === "cancelling"
                                    ? "Stopping download…"
                                    : `Downloading ${meta.modelFile}`}
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
                        {eventForTier(id)?.attempt_id &&
                          eventForTier(id)?.phase !== "committing" && (
                            <button
                              type="button"
                              className="btn btn-ghost !py-1.5 !px-3 !text-xs"
                              disabled={eventForTier(id)?.phase === "cancelling"}
                              onClick={() => {
                                void api
                                  .cancelIntelligenceDownload(id, eventForTier(id)!.attempt_id!)
                                  .catch((failure) =>
                                    intelligence.notifyToast(
                                      "error",
                                      `Could not cancel the download: ${String(failure)}`,
                                    ),
                                  );
                              }}
                            >
                              Cancel download
                            </button>
                          )}
                      </div>
                    ) : isWeightsOnly(id) || !runtimeInstalled ? (
                      <>
                        <span className="inline-flex items-center gap-1.5 text-xs text-warning font-medium">
                          <AlertTriangle className="w-3.5 h-3.5" />
                          {isWeightsOnly(id)
                            ? "Weights only — runtime missing"
                            : "Install the writing runtime first"}
                        </span>
                        <div className="flex-1" />
                        {runtimeDownloadActive ? (
                          <div className="flex items-center gap-2">
                            <span className="inline-flex items-center gap-1.5 text-2xs text-accent font-medium">
                              <Loader2 className="w-3 h-3 animate-spin" />
                              {runtimeDownload?.phase === "cancelling"
                                ? "Stopping setup…"
                                : "Installing runtime…"}{" "}
                              {runtimeDownload?.progress_pct ?? 0}%
                            </span>
                            {runtimeDownload?.attempt_id &&
                              runtimeDownload.phase !== "activating" && (
                                <button
                                  type="button"
                                  className="btn btn-ghost btn-sm"
                                  disabled={runtimeDownload.phase === "cancelling"}
                                  onClick={() => {
                                    api
                                      .cancelRuntimeDownload(runtimeDownload.attempt_id!)
                                      .catch((error) =>
                                        intelligence.notifyToast("error", String(error)),
                                      );
                                  }}
                                >
                                  Cancel runtime setup
                                </button>
                              )}
                          </div>
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
      </details>

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
          <p className="text-sm font-semibold text-ink">Try your cleanup settings</p>
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
          aria-label="Sample transcript"
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
              {previewUsed
                ? formattingEnabled
                  ? `LLM formatting · ${previewLatency}ms`
                  : `Cleaned in ${previewLatency}ms`
                : formattingEnabled
                  ? `Deterministic fallback · ${previewLatency}ms`
                  : `Basic cleanup · ${previewLatency}ms`}
              {previewUsed && previewModel && previewModel !== "none" && ` with ${previewModel}`}
            </span>
          )}
        </div>
        {previewNotice && (
          <p role="status" className="text-xs text-warning leading-relaxed">
            {formattingEnabled
              ? "Intelligent formatting was not applied."
              : "AI rewrite was not applied."}{" "}
            {previewNotice}
          </p>
        )}
        <div className="rounded-lg bg-surface-2 border border-line px-3 py-2.5">
          <p className="text-2xs text-muted mb-1">Output</p>
          <p className="text-sm text-ink leading-6 whitespace-pre-wrap">
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
