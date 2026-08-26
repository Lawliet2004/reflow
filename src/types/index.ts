export type AppState =
  | "UNINITIALIZED"
  | "INITIALIZING"
  | "IDLE"
  | "LOADING_MODEL"
  | "READY"
  | "RECORDING"
  | "PROCESSING"
  | "INJECTING"
  | "ERROR"
  | "UPDATING";

export type ProcessingMode = "raw" | "smart" | "flow";
export type DictationMode = "normal" | "coding" | "email" | "chat" | "notes";
export type CleanupLevel = "raw" | "light" | "medium" | "high";
export type FlowModel = "lfm2.5-1.2b" | "qwen3.5-2b" | "none";
export type IntelligenceTier = "raw_verbatim" | "smart_flow" | "deep_context";

export interface TierMetadata {
  id: IntelligenceTier;
  label: string;
  modelId: FlowModel;
  modelFile: string;
  tagline: string;
  description: string;
  latencyEstimate: string;
  downloadSizeMB: number;
  vramRequiredMB: number;
  ramRequiredMB: number;
  badgeText: string;
  recommendedHardware: "all" | "cpu_fast" | "gpu_recommended";
}

export const INTELLIGENCE_TIERS: Record<IntelligenceTier, TierMetadata> = {
  raw_verbatim: {
    id: "raw_verbatim",
    label: "Exact Voice",
    modelId: "none",
    modelFile: "",
    tagline: "Zero delay, literal words",
    description: "Direct speech-to-text output. No LLM rewriting, no rephrasing, 100% literal words. Best for coding CLI, terminal, or exact records.",
    latencyEstimate: "Zero latency",
    downloadSizeMB: 0,
    vramRequiredMB: 0,
    ramRequiredMB: 0,
    badgeText: "ZERO DELAY",
    recommendedHardware: "all",
  },
  smart_flow: {
    id: "smart_flow",
    label: "Smart Flow",
    modelId: "lfm2.5-1.2b",
    modelFile: "LFM2.5-1.2B-Instruct-Q4_K_M.gguf",
    tagline: "Fast, natural cleanup and auto-repair",
    description: "Drops 'ums', stutters, and spoken self-corrections while preserving your exact voice.",
    latencyEstimate: "Ultra-Fast (<100ms)",
    downloadSizeMB: 750,
    vramRequiredMB: 900,
    ramRequiredMB: 850,
    badgeText: "RECOMMENDED",
    recommendedHardware: "cpu_fast",
  },
  deep_context: {
    id: "deep_context",
    label: "Deep Context & Languages",
    modelId: "qwen3.5-2b",
    modelFile: "Qwen3.5-2B-Instruct-Q4_K_M.gguf",
    tagline: "Advanced multilingual reasoning",
    description: "Handles 201 languages, mixed dialects, code-mixing, and formats complex technical dictation.",
    latencyEstimate: "Smooth (~350ms)",
    downloadSizeMB: 1400,
    vramRequiredMB: 1600,
    ramRequiredMB: 1800,
    badgeText: "PRO",
    recommendedHardware: "gpu_recommended",
  },
};

export function flowModelForTier(tier: IntelligenceTier | undefined): FlowModel {
  if (tier === "deep_context") return "qwen3.5-2b";
  if (tier === "raw_verbatim") return "none";
  return "lfm2.5-1.2b";
}

export function tierForCleanupLevel(level: CleanupLevel | undefined): IntelligenceTier {
  if (level === "raw" || level === "light") return "raw_verbatim";
  if (level === "high") return "deep_context";
  return "smart_flow";
}
export type TranscriptStyle = "faithful" | "neutral" | "decisive" | "email" | "chat";
export type HistoryRetention = "disabled" | "1_day" | "7_days" | "30_days" | "90_days" | "forever";
export type OverlayPosition = "bottom_center" | "top_center" | "bottom_right" | "top_right";
export type ComputeBackend = "auto" | "cpu" | "gpu";
export type AppTheme = "system" | "light" | "dark";
export type AccentColor =
  | "sky"
  | "indigo"
  | "emerald"
  | "amber"
  | "rose"
  | "violet"
  | "graphite";
export type HudScale = "compact" | "standard" | "large";
export type WaveformStyle = "bars" | "pulse" | "minimal";
export type UIFontScale = "compact" | "normal" | "roomy";

export interface AppSettings {
  hotkey: string;
  push_to_talk: boolean;
  auto_stop_silence_ms: number;
  max_duration_sec: number;
  language: string; // 'auto', 'en', 'hi', 'bn'
  auto_detect_language: boolean;
  microphone_device_id: string | null;
  input_gain: number; // 0.1 to 3.0
  vad_sensitivity: number; // 0.0 to 1.0 (threshold)
  processing_mode: ProcessingMode;
  dictation_mode: DictationMode;
  compute_backend: ComputeBackend;
  asr_model: string; // "0.6b" | "1.7b"
  /** ASR weight precision. "auto" picks the best fit for the GPU, the
   *  other values force a specific mode. Persisted across launches. */
  asr_precision: "auto" | "int4" | "int8" | "bf16";
  /** Number of Stage 2 LLM transformer layers to offload to the GPU.
   *  -1 = auto (binary 0/99 driven by compute_backend);
   *   0 = CPU only;
   *   N>0 = offload N layers (partial offload). */
  flow_n_gpu_layers: number;
  keep_model_loaded: boolean;
  history_retention: HistoryRetention;
  overlay_position: OverlayPosition;
  overlay_theme: "dark" | "light" | "auto";
  app_theme: AppTheme;
  accent_color: AccentColor;
  hud_scale: HudScale;
  waveform_style: WaveformStyle;
  reduce_motion: boolean;
  ui_font_scale: UIFontScale;
  developer_mode: boolean;
  cleanup_level: CleanupLevel;
  intelligence_tier: IntelligenceTier;
  flow_model: FlowModel;
  style: TranscriptStyle;
  auto_style_from_app: boolean;
  active_profile: string;
  launch_at_startup: boolean;
  start_minimized: boolean;
  offline_mode: boolean;
  spoken_punctuation_enabled: boolean;
  filler_removal_enabled: boolean;
  clipboard_restore_enabled: boolean;
  dictionary_terms: DictionaryTerm[];
  custom_replacements: CustomReplacement[];
  api_enabled: boolean;
  api_bind: "localhost" | "lan";
  api_port: number;
  api_mdns: boolean;
  api_inject_default: boolean;
}

export interface AudioDevice {
  id: string;
  name: string;
  is_default: boolean;
  sample_rate: number;
  channels: number;
}

export interface HistoryEntry {
  id: string;
  created_at: string;
  duration_ms: number;
  language: string;
  raw_transcript: string;
  smart_transcript?: string;
  final_transcript: string;
  rewriter_used?: boolean;
  application_name: string;
  application_process: string;
  word_count: number;
  character_count: number;
  model_version: string;
  processing_mode: ProcessingMode;
}

export interface DictionaryTerm {
  id: string;
  term: string;
  preferred_spelling: string;
  category: string;
}

export interface CustomReplacement {
  id: string;
  before: string;
  after: string;
  enabled: boolean;
}

export interface LatencyMetrics {
  hotkey_to_recording_ms: number;
  recording_to_first_audio_ms: number;
  audio_to_first_partial_ms: number;
  speech_end_to_final_ms: number;
  final_to_injection_ms: number;
  total_duration_ms: number;
  rewrite_ms?: number;
  last_updated: string;
}

export interface SystemMetrics {
  cpu_usage_pct: number;
  app_ram_mb: number;
  model_ram_mb: number;
  total_ram_mb: number;
  vram_mb: number;
  gpu_name: string;
  model_loaded: boolean;
  backend_name: string;
  os_name: string;
  session: string;
}

export interface PlatformInfo {
  os: string;
  session: string;
  default_hotkey: string;
  data_dir: string;
  logs_dir: string;
  hotkey_error: string | null;
  injection_notes: string;
}

export interface InjectionFeedback {
  pasted: boolean;
  fallback_copy: boolean;
  paste_chord: string;
  process_name: string;
  message: string;
}

export interface ModelStatus {
  installed: boolean;
  loaded: boolean;
  version: string;
  name: string;
  size_bytes: number;
  download_progress_pct: number;
  download_speed_mbps: number;
  backend: string;
  is_downloading: boolean;
  is_loading?: boolean;
  error: string | null;
  /** True when nvidia-smi reports a non-CPU device on this machine. */
  gpu_available?: boolean;
  /** Display name of the detected GPU, or "CPU" if none. */
  gpu_name?: string;
  /** True iff the ASR sidecar's torch is CUDA-enabled. */
  cuda_available?: boolean;
  /** The CUDA version the loaded torch was built against. */
  torch_cuda_version?: string | null;
  /** Pre-formatted `pip install` command shown when a GPU is present
   *  but the sidecar's torch is CPU-only. */
  asr_gpu_hint?: string | null;
}

export interface FlowStatus {
  active_tier: IntelligenceTier;
  active_model: FlowModel;
  /** True when the GGUF weights for the active flow model are on disk. */
  installed: boolean;
  /** True when the `llama-server` runtime binary is on disk. */
  runtime_installed: boolean;
  ready: boolean;
  backend: string;
  is_loading: boolean;
  is_downloading: boolean;
  download_progress_pct: number;
  /** Effective execution mode of the running `llama-server` child. */
  mode?: "cpu" | "gpu";
  /** Actual `--n-gpu-layers` value the running child was launched with. */
  n_gpu_layers?: number;
  /** Approximate VRAM in use by the running child, in MB. */
  vram_used_mb?: number;
}

/** Per-tier install state. Returned by `get_intelligence_tiers`. */
export interface IntelligenceTierState {
  tier: IntelligenceTier;
  model_id: FlowModel;
  /** True when the GGUF weights for this tier are on disk AND the
   *  `llama-server` runtime is on disk. The Download button should be hidden
   *  when this is `true`. */
  installed: boolean;
  /** True while a download is in flight for this tier. */
  downloading: boolean;
}

/** Phases of the `llama-server` runtime downloader. */
export type RuntimeDownloadPhase =
  | "starting"
  | "downloading"
  | "verifying"
  | "extracting"
  | "complete"
  | "error";

/** Payload for the `runtime:download-progress` event. */
export interface RuntimeDownloadEvent {
  /** Pinned llama.cpp release tag, e.g. "b10621". */
  version: string;
  progress_pct: number;
  speed_mbps: number;
  phase: RuntimeDownloadPhase;
  error?: string;
  path?: string;
  approx_bytes: number;
  /** Free-form label, e.g. "Vulkan" or "CPU". */
  kind_label?: string;
}

export function isModelReady(status: ModelStatus | null | undefined): boolean {
  if (!status?.loaded) return false;
  if (status.is_loading) return false;
  const backend = (status.backend || "").toLowerCase();
  if (backend.includes("loading") || backend.includes("not loaded")) return false;
  return true;
}

export function isModelLoading(status: ModelStatus | null | undefined): boolean {
  if (!status) return true;
  if (status.is_downloading) return false;
  return !isModelReady(status) && !status.error;
}

export interface StreamingTranscriptPayload {
  committed_prefix: string;
  mutable_suffix: string;
  full_text: string;
  language: string;
  audio_level: number;
  stage?: string;
}

export interface PairedDevice {
  id: string;
  name: string;
  created_at: string;
}

export interface ApiStatus {
  enabled: boolean;
  running: boolean;
  bind: string;
  port: number;
  listen_addrs: string[];
  pairing_code: string | null;
  pairing_expires_in_sec: number | null;
  qr_svg: string | null;
  pair_uri: string | null;
  devices: PairedDevice[];
  warning: string;
}

export function inferCleanupLevel(settings: Pick<AppSettings, "cleanup_level" | "processing_mode">): CleanupLevel {
  if (settings.cleanup_level) return settings.cleanup_level;
  if (settings.processing_mode === "raw") return "raw";
  if (settings.processing_mode === "flow") return "medium";
  return "light";
}

export function processingModeForCleanup(level: CleanupLevel): ProcessingMode {
  if (level === "raw") return "raw";
  if (level === "medium" || level === "high") return "flow";
  return "smart";
}

export function normalizeSettings(settings: AppSettings): AppSettings {
  const intelligence_tier = settings.intelligence_tier ?? tierForCleanupLevel(inferCleanupLevel(settings));
  return {
    ...settings,
    cleanup_level: inferCleanupLevel(settings),
    intelligence_tier,
    flow_model: flowModelForTier(intelligence_tier),
    asr_precision: settings.asr_precision ?? "auto",
    flow_n_gpu_layers: settings.flow_n_gpu_layers ?? -1,
    style: settings.style ?? "neutral",
    auto_style_from_app: settings.auto_style_from_app ?? true,
    developer_mode: settings.developer_mode ?? false,
    app_theme: settings.app_theme ?? "system",
    accent_color: settings.accent_color ?? "sky",
    hud_scale: settings.hud_scale ?? "standard",
    waveform_style: settings.waveform_style ?? "bars",
    reduce_motion: settings.reduce_motion ?? false,
    ui_font_scale: settings.ui_font_scale ?? "normal",
    overlay_theme: settings.overlay_theme ?? "dark",
    overlay_position: settings.overlay_position ?? "bottom_center",
  };
}

export interface InjectionResult {
  success: boolean;
  text: string;
  target_app: string;
  duration_ms: number;
  error?: string;
}
