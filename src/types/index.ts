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
export type FlowModel = "qwen3.5-0.8b" | "qwen3.5-2b" | "none";
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

/**
 * User-facing tier metadata.
 *
 * The labels deliberately match the vocabulary the HUD badge and the tray item
 * use. Three different names for the same axis had grown up independently —
 * "Exact Voice" / "Smart Flow" / "Deep Context" here, Fast/Polished on the HUD,
 * and Fast/Balanced/Accurate on the preset control — so the same choice read as
 * three unrelated settings. The stored ids are untouched, so nothing migrates.
 */
export const INTELLIGENCE_TIERS: Record<IntelligenceTier, TierMetadata> = {
  raw_verbatim: {
    id: "raw_verbatim",
    label: "Fast",
    modelId: "none",
    modelFile: "",
    tagline: "Your exact words, no LLM",
    description:
      "Direct speech-to-text output. No LLM rewriting, no rephrasing, 100% literal words. Best for coding CLI, terminal, or exact records.",
    latencyEstimate: "No polish step",
    downloadSizeMB: 0,
    vramRequiredMB: 0,
    ramRequiredMB: 0,
    badgeText: "FASTEST",
    recommendedHardware: "all",
  },
  smart_flow: {
    id: "smart_flow",
    label: "Polished",
    modelId: "qwen3.5-0.8b",
    modelFile: "Qwen3.5-0.8B-Q4_K_M.gguf",
    tagline: "Fast, natural cleanup and auto-repair",
    description:
      "Drops 'ums', stutters, and spoken self-corrections while preserving your exact voice.",
    latencyEstimate: "Not measured on this computer",
    downloadSizeMB: 508,
    vramRequiredMB: 900,
    ramRequiredMB: 850,
    badgeText: "RECOMMENDED",
    recommendedHardware: "cpu_fast",
  },
  deep_context: {
    id: "deep_context",
    label: "Polished · Deep",
    modelId: "qwen3.5-2b",
    // No `-Instruct` infix: the unsloth GGUFs are not named that way, and the
    // Rust registry has a test asserting it.
    modelFile: "Qwen3.5-2B-Q4_K_M.gguf",
    tagline: "Advanced multilingual reasoning",
    description:
      "Multilingual cleanup for supported speech languages, including mixed-language technical dictation.",
    latencyEstimate: "Not measured on this computer",
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
  return "qwen3.5-0.8b";
}

/**
 * Tier implied by a cleanup level, for documents with no explicit tier.
 *
 * Only `raw` implies verbatim. `light` used to map here too, which silently
 * disabled polishing for anyone who wanted a gentle deterministic cleanup — the
 * same conflation that was removed from the Rust `tier_for_cleanup`, where
 * `cleanup_level` describes Stage 1 aggressiveness and never gates Stage 2.
 */
export function tierForCleanupLevel(level: CleanupLevel | undefined): IntelligenceTier {
  if (level === "raw") return "raw_verbatim";
  if (level === "high") return "deep_context";
  return "smart_flow";
}

/**
 * Whether the LLM polish pass will run.
 *
 * Mirrors `ResolvedIntent::run_llm`: the tier decides, with `cleanup_level: raw`
 * as the one override, because asking for verbatim words and then rewriting them
 * would contradict the request.
 */
export function polishEnabledFor(settings: {
  intelligence_tier?: IntelligenceTier | string;
  cleanup_level?: CleanupLevel | string;
}): boolean {
  const tier = (settings.intelligence_tier ?? "smart_flow").toString().toLowerCase();
  const cleanup = (settings.cleanup_level ?? "").toString().toLowerCase();
  if (tier === "raw_verbatim") return false;
  if (cleanup === "raw") return false;
  return flowModelForTier(tier as IntelligenceTier) !== "none";
}
export type TranscriptStyle = "faithful" | "neutral" | "decisive" | "email" | "chat";
export type HistoryRetention = "disabled" | "1_day" | "7_days" | "30_days" | "90_days" | "forever";
export type AudioRetention = "disabled" | "1_day" | "7_days" | "30_days" | "forever";
export type OverlayPosition = "bottom_center" | "top_center" | "bottom_right" | "top_right";
export type ComputeBackend = "auto" | "cpu" | "gpu";
export type AppTheme = "system" | "light" | "dark";
export type AccentColor = "sky" | "indigo" | "emerald" | "amber" | "rose" | "violet" | "graphite";
export type HudScale = "compact" | "standard" | "large";
export type WaveformStyle = "bars" | "pulse" | "minimal";
export type UIFontScale = "compact" | "normal" | "roomy";

/**
 * ASR and refinement are separate placement decisions.
 *
 * A single `compute_backend` used to govern both, which made "ASR on the GPU,
 * refinement on the CPU" — the correct configuration on a 4 GB card —
 * impossible to express.
 */
export interface AsrSettings {
  runtime: "python" | "native";
  /** Manifest id, e.g. `"0.6b"`. */
  model: string;
  device: "auto" | "cpu" | "cuda";
  precision: "auto" | "bf16" | "int8" | "int4";
  keep_loaded: boolean;
}

export interface RefinementSettings {
  /** Manifest id, or `"none"` for ASR-only. */
  model: FlowModel;
  device: "auto" | "cpu" | "cuda" | "vulkan";
  /** `-1` derive from `device`, `0` force CPU, `N` offload N layers. */
  gpu_layers: number;
  /** `--ctx-size`. Longer input is chunked or refused, never truncated. */
  context_size: number;
  keep_warm: boolean;
  /** Fall back to deterministic text after this long. */
  deadline_ms: number;
}

export interface StreamingSettings {
  mode: "auto" | "off" | "chunk_on_silence";
  /** Minimum silence that counts as a segment boundary. */
  silence_boundary_ms: number;
  /** Force a boundary after this much continuous speech. */
  max_segment_seconds: number;
}

export interface MemoryPolicySettings {
  /** `0` derives the reserve from the detected hardware. */
  vram_reserve_mb: number;
  /** `0` never unloads. */
  unload_idle_after_seconds: number;
  /** Off by default: an unmeasured GPU refinement load can spill silently. */
  allow_gpu_refinement: boolean;
}

export interface AppSettings {
  sounds_enabled?: boolean;
  sounds_volume?: number;
  history_encryption?: boolean;
  excluded_apps?: string[];
  duck_media?: boolean;
  follow_default_mic?: boolean;
  voice_commands_enabled?: boolean;
  min_dictation_ms?: number;
  power_policy?: { unload_on_battery: boolean; battery_preset: string | null };
  hud_contrast?: "standard" | "high";
  paste_delay_ms?: number;
  inject_method?: "paste" | "type";
  append_space?: boolean;
  capitalize_first?: boolean;
  meeting_mode?: boolean;
  meeting_monitor_device_id?: string | null;

  notes_folder?: string;
  dictionary_suggestions?: { before: string; after: string; frequency: number }[];
  dismissed_corrections?: string[];
  hotkeys?: Hotkeys;
  output_action?: OutputAction;
  send_key?: SendKey;
  modes?: Mode[];
  default_mode_id?: string;
  mode_triggers_enabled?: boolean;
  snippets?: Snippet[];
  hotkey: string;
  push_to_talk: boolean;
  auto_stop_silence_ms: number;
  max_duration_sec: number;
  language: string; // 'auto' or a code from the shared dictation-language catalogue
  auto_detect_language: boolean;
  microphone_device_id: string | null;
  input_gain: number; // 0.1 to 3.0
  vad_sensitivity: number; // 0.0 to 1.0 (threshold)
  processing_mode: ProcessingMode | "command" | "assistant" | "file";
  dictation_mode: DictationMode;
  /** Performance intent. `"custom"` is never overwritten by the resolver. */
  preset: Preset;
  /** Speech-model placement, independent of {@link AppSettings.refinement}. */
  asr: AsrSettings;
  /** Refinement-model placement, independent of {@link AppSettings.asr}. */
  refinement: RefinementSettings;
  streaming: StreamingSettings;
  memory_policy: MemoryPolicySettings;
  history_retention: HistoryRetention;
  audio_retention: AudioRetention;
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
  application_profiles?: ApplicationProfile[];
  custom_replacements: CustomReplacement[];
  api_enabled: boolean;
  api_bind: "localhost" | "lan";
  api_port: number;
  api_inject_default: boolean;
}

export type SessionIntent = "dictate" | "command" | "assistant" | "note";
export type SendKey = "enter" | "shift_enter" | "ctrl_enter";
export type OutputAction =
  | { type: "paste" | "paste_enter" | "copy" | "hud" }
  | { type: "append_file"; path: string }
  | { type: "run_command"; template: string };
export interface Hotkeys {
  dictation: string;
  command: string | null;
  assistant: string | null;
  note: string | null;
  cancel: "Escape" | null;
}
export interface Snippet {
  id: string;
  trigger: string;
  expansion: string;
  enabled: boolean;
}
export interface Mode {
  id: string;
  name: string;
  icon?: string | null;
  hotkey: string | null;
  triggers: { apps: string[]; spoken: string[] };
  language: string | null;
  dictation_mode: DictationMode;
  cleanup_level: CleanupLevel;
  intelligence_tier: IntelligenceTier;
  style: TranscriptStyle;
  custom_instructions: string;
  context: { selected_text: boolean; clipboard: boolean; window_title: boolean };
  output: OutputAction;
  send_key: SendKey;
  enabled: boolean;
  translate_to?: string | null;
}
export function defaultModes(): Mode[] {
  const dictation: Mode = {
    id: "dictation",
    name: "Dictation",
    hotkey: null,
    triggers: { apps: [], spoken: [] },
    language: null,
    dictation_mode: "normal",
    cleanup_level: "light",
    intelligence_tier: "smart_flow",
    style: "neutral",
    custom_instructions: "",
    context: { selected_text: false, clipboard: false, window_title: false },
    output: { type: "paste" },
    send_key: "enter",
    enabled: true,
    translate_to: null,
  };
  return [
    dictation,
    { ...structuredClone(dictation), id: "message", name: "Message", style: "chat" },
    { ...structuredClone(dictation), id: "email", name: "Email", style: "email" },
    {
      ...structuredClone(dictation),
      id: "coding",
      name: "Coding",
      dictation_mode: "coding",
      cleanup_level: "raw",
      intelligence_tier: "raw_verbatim",
    },
  ];
}

export interface AudioDevice {
  is_monitor?: boolean;
  id: string;
  name: string;
  is_default: boolean;
  sample_rate: number;
  channels: number;
}

export interface HistoryEntry {
  kind?: string;
  source?: string;
  pinned?: boolean;
  tags?: string;
  command_input?: string | null;
  audio_available?: boolean;
  audio_expires_at?: number | null;
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
  processing_mode: ProcessingMode | "command" | "assistant" | "file";
}

export interface DictionaryTerm {
  id: string;
  term: string;
  preferred_spelling: string;
  category: string;
}
export interface ApplicationProfile {
  process: string;
  style: TranscriptStyle;
  dictionary_terms: DictionaryTerm[];
}

export interface CustomReplacement {
  id: string;
  before: string;
  after: string;
  enabled: boolean;
}

/** Timing for one ASR segment submission. */
export interface SegmentTiming {
  sequence_id: number;
  submitted_at_ms: number;
  completed_at_ms: number | null;
  compute_ms: number | null;
  audio_ms: number;
  rtf: number | null;
}

export interface LatencyMetrics {
  dropped_audio_chunks?: number;
  failed_audio_pushes?: number;
  peak_audio_queue_chunks?: number;
  llm_was_warm?: boolean;
  llm_generation?: {
    prompt_tokens: number;
    output_tokens: number;
    prompt_ms: number | null;
    decode_ms: number | null;
  } | null;
  hotkey_to_recording_ms: number;
  recording_to_first_audio_ms: number;
  audio_to_first_partial_ms: number;
  speech_end_to_final_ms: number;
  final_to_injection_ms: number;
  /** Cold-starting the refinement runtime inside the stop path. */
  llm_startup_ms: number;
  /** Deterministic Stage 1 only. */
  formatting_ms: number;
  /** Stage 2 LLM only; 0 when no rewrite was dispatched. */
  rewrite_ms: number;
  /** Whether the injected text actually came from the LLM. */
  rewrite_applied: boolean;
  /** The headline number: key release to text in the target app. */
  release_to_inserted_ms: number;
  total_duration_ms: number;
  audio_duration_ms: number;
  /** ASR compute time / audio duration. Above 1.0 is slower than real time. */
  rtf: number;
  segments: SegmentTiming[];
  last_updated: string;
}

/** Rolling p50/p95 across recent dictations in this app session. */
export interface LatencyPercentiles {
  samples: number;
  hotkey_to_recording_p50_ms: number;
  hotkey_to_recording_p95_ms: number;
  speech_end_to_final_p50_ms: number;
  speech_end_to_final_p95_ms: number;
  rewrite_p50_ms: number;
  rewrite_p95_ms: number;
  release_to_inserted_p50_ms: number;
  release_to_inserted_p95_ms: number;
  rtf_p50: number;
  rtf_p95: number;
  /** Fraction of dictations whose text came from the LLM. */
  llm_applied_rate: number;
}

/** Everything the developer diagnostics view needs in one round trip. */
export interface LatencyReport {
  last: LatencyMetrics;
  percentiles: LatencyPercentiles;
  recent: LatencyMetrics[];
}

export interface AsrBenchmarkMetrics {
  model_id: string;
  device: string;
  precision: string;
  load_ms: number;
  warmup_rtf: number | null;
  /** null = not measured on this machine (never a fabricated value). */
  average_inference_ms: number | null;
  corpus_wer: number | null;
  corpus_cer: number | null;
  spill_detected: boolean;
}

export interface RefinementBenchmarkMetrics {
  model_id: string;
  device: string;
  load_ms: number | null;
  tokens_per_second: number | null;
  safety_pass_rate: number | null;
  average_latency_ms: number | null;
}

export interface FullBenchmarkReport {
  timestamp: string;
  asr?: AsrBenchmarkMetrics;
  refinement?: RefinementBenchmarkMetrics;
  hardware_summary: string;
}

export interface HistoryQuery {
  kind?: string;
  application_process?: string;
  pinned_only?: boolean;
  tag?: string;
  query?: string;
  from?: string | null;
  until?: string | null;
  limit?: number;
  offset?: number;
}

export interface HistoryPage {
  entries: HistoryEntry[];
  total: number;
  next_offset: number | null;
  recovery_notice: string | null;
}

export interface RuntimeEntry {
  id: string;
  version: string;
  kind: string;
  binary_path: string;
  active: boolean;
  rollback_available: boolean;
  healthy: boolean;
  error: string | null;
}

export interface MicrophoneHealth {
  device: string;
  duration_ms: number;
  peak: number;
  rms: number;
  clipped_pct: number;
  dropped_chunks: number;
  assessment: string;
}

export interface RecognitionTest {
  text: string;
  language: string;
  health: MicrophoneHealth;
}

export interface SystemMetrics {
  cpu_usage_pct: number;
  app_ram_mb: number;
  model_ram_mb: number;
  /** Physical system RAM. Previously this field held *used* RAM. */
  total_ram_mb: number;
  used_ram_mb: number;
  available_ram_mb: number;
  /** VRAM in use. Prefer the explicit fields below. */
  vram_mb: number;
  total_vram_mb: number;
  used_vram_mb: number;
  /** The only figure an admission decision may use. */
  free_vram_mb: number;
  gpu_name: string;
  gpu_vendor: string;
  /** A GPU was enumerated. Distinct from `cuda_available`. */
  gpu_present: boolean;
  /** `torch.cuda.is_available()` in the ASR sidecar. */
  cuda_available: boolean;
  vulkan_available: boolean;
  cpu_model: string;
  physical_cores: number;
  logical_cores: number;
  asr_ram_mb: number;
  refinement_ram_mb: number;
  model_loaded: boolean;
  backend_name: string;
  os_name: string;
  session: string;
}

export type GpuVendor = "nvidia" | "amd" | "intel" | "apple" | "unknown";

export interface GpuInfo {
  index: number;
  name: string;
  vendor: GpuVendor;
  total_vram_mb: number;
  used_vram_mb: number;
  free_vram_mb: number;
  driver_version: string | null;
  compute_capability: string | null;
}

export interface CudaStatus {
  nvidia_gpu_present: boolean;
  driver_present: boolean;
  driver_cuda_version: string | null;
  torch_cuda_available: boolean;
  torch_cuda_version: string | null;
}

export interface VulkanStatus {
  available: boolean;
  api_version: string | null;
  devices: string[];
}

export interface CpuInfo {
  model: string;
  physical_cores: number | null;
  logical_cores: number;
  load_pct: number;
}

export interface RamInfo {
  total_mb: number;
  used_mb: number;
  available_mb: number;
  app_mb: number;
  asr_mb: number;
  refinement_mb: number;
}

export type Precision = "fp32" | "bf16" | "int8" | "int4";
export type ComputeDevice = "cpu" | "cuda" | "vulkan";
export type RuntimeKind = "python_asr" | "llama_server" | "native_asr";
export type Preset = "auto" | "fast" | "balanced" | "accurate" | "custom";

export interface ModelManifest {
  id: string;
  label: string;
  runtime: RuntimeKind;
  repo: string;
  revision: string;
  filename: string;
  dir_name: string;
  sha256: string;
  download_bytes: number;
  params: number;
  precisions: Precision[];
  devices: ComputeDevice[];
  /** Only languages that have passed their own validation corpus. */
  languages: string[];
  overhead_mb: number;
}

export interface LoadAttempt {
  device: ComputeDevice;
  precision: Precision;
  expected_peak_mb: number;
  /** `true` when this rung came from a measurement, not an estimate. */
  measured: boolean;
}

/** Machine-readable explanation for a resolver decision. */
export interface ProfileReason {
  code: string;
  detail: string;
}

/**
 * A requested configuration the resolver cannot honor at all — e.g. a Custom
 * precision the model does not support. Unlike a reason, this must be shown
 * as an error: the load will fail if attempted.
 */
export interface ProfileError {
  code: string;
  detail: string;
  requested: string;
  supported: string[];
}

export interface ProfileOverrides {
  asr_model?: string | null;
  asr_device?: ComputeDevice | null;
  asr_precision?: Precision | null;
  refinement_model?: string | null;
  refinement_device?: ComputeDevice | null;
  refinement_gpu_layers?: number | null;
  language?: string | null;
  force_streaming?: boolean | null;
}

export interface ResolvedProfile {
  preset: Preset;
  asr_model: string;
  asr_device: ComputeDevice;
  asr_precision: Precision;
  asr_attempts: LoadAttempt[];
  /** `null` means ASR-only, which is a first-class mode rather than a failure. */
  refinement_model: string | null;
  refinement_device: ComputeDevice;
  refinement_gpu_layers: number;
  language: string;
  streaming_enabled: boolean;
  inference_threads: number;
  reasons: ProfileReason[];
  errors?: ProfileError[];
}

export interface RuntimePlan {
  preset: Preset;
  asr_model: string;
  asr_device: string;
  asr_precision: string;
  refinement_model: string | null;
  refinement_device: string;
  refinement_gpu_layers: number;
  context_size: number;
  inference_threads: number;
  keep_asr_loaded: boolean;
  keep_refinement_warm: boolean;
  reasons: ProfileReason[];
  error: string | null;
}

export function isAsrOnly(profile: ResolvedProfile): boolean {
  return profile.refinement_model === null;
}

export function findReason(profile: ResolvedProfile, code: string): ProfileReason | undefined {
  return profile.reasons.find((r) => r.code === code);
}

/** The full, honest picture of the machine. */
export interface Capabilities {
  gpus: GpuInfo[];
  cuda: CudaStatus;
  vulkan: VulkanStatus;
  cpu: CpuInfo;
  ram: RamInfo;
  os_name: string;
  probed_at: string;
}

/**
 * `true` when an NVIDIA GPU exists but Python cannot use it — a fixable state
 * that must never be reported as "no GPU".
 */
export function gpuPresentButCudaUnavailable(caps: Capabilities): boolean {
  return caps.cuda.nvidia_gpu_present && !caps.cuda.torch_cuda_available;
}

/** The device a GPU workload would target: most free VRAM among NVIDIA. */
export function primaryGpu(caps: Capabilities): GpuInfo | null {
  const nvidia = caps.gpus.filter((g) => g.vendor === "nvidia");
  if (nvidia.length > 0) {
    return nvidia.reduce((best, g) => (g.free_vram_mb > best.free_vram_mb ? g : best));
  }
  return caps.gpus[0] ?? null;
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
  /**
   * Why the running ASR model differs from the one selected in Settings.
   *
   * Set when the requested model could not fit free VRAM and a smaller one was
   * loaded to keep transcription on the GPU. Absent when they match.
   */
  asr_selection_notice?: string | null;
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
  /** True when this tier's model weights are downloaded, independently of the runtime. */
  weights_installed?: boolean;
  /** True when the GGUF weights for this tier are on disk AND the
   *  `llama-server` runtime is on disk. The Download button should be hidden
   *  when this is `true`. */
  installed: boolean;
  /** True while a download is in flight for this tier. */
  downloading: boolean;
}

/** Phases of the `llama-server` runtime downloader. */
export type RuntimeDownloadPhase =
  "starting" | "downloading" | "verifying" | "extracting" | "complete" | "error";

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
  if (!status.installed || status.is_downloading) return false;
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
  permissions?: { stream: boolean; history: boolean; injection: boolean };
}

export interface CalibrationCandidate {
  id: string;
  label: string;
  median_ms: number;
  p95_ms: number;
  wer: number;
  cer: number;
  eligible: boolean;
  error: string | null;
  observed_vram_mb?: number;
  minimum_ram_mb?: number;
  transcript?: string;
  output?: string;
}
export interface CalibrationStatus {
  running: boolean;
  phase: string;
  candidates: CalibrationCandidate[];
  winner_id: string | null;
  error: string | null;
  language: string;
  reference: string;
}

export interface ApiStatus {
  certificate_sha256?: string | null;
  transport?: string;
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

export function inferCleanupLevel(
  settings: Pick<AppSettings, "cleanup_level" | "processing_mode">,
): CleanupLevel {
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

export const DEFAULT_ASR_SETTINGS: AsrSettings = {
  runtime: "python",
  model: "0.6b",
  device: "auto",
  precision: "auto",
  keep_loaded: true,
};

export const DEFAULT_REFINEMENT_SETTINGS: RefinementSettings = {
  model: "qwen3.5-0.8b",
  device: "auto",
  gpu_layers: -1,
  context_size: 1024,
  keep_warm: true,
  deadline_ms: 12_000,
};

export const DEFAULT_STREAMING_SETTINGS: StreamingSettings = {
  mode: "auto",
  silence_boundary_ms: 400,
  max_segment_seconds: 12,
};

export const DEFAULT_MEMORY_POLICY: MemoryPolicySettings = {
  vram_reserve_mb: 0,
  unload_idle_after_seconds: 0,
  allow_gpu_refinement: false,
};

/**
 * Defensive boundary between the backend payload and the UI.
 *
 * The backend already normalizes, but a settings document written by an older
 * build can arrive here with sections missing entirely, and a component that
 * reads `settings.asr.device` must never see `undefined`.
 */
export function normalizeSettings(settings: AppSettings): AppSettings {
  const intelligence_tier =
    settings.intelligence_tier ?? tierForCleanupLevel(inferCleanupLevel(settings));
  const flow_model = flowModelForTier(intelligence_tier);
  return {
    ...settings,
    sounds_enabled: settings.sounds_enabled ?? true,
    sounds_volume: settings.sounds_volume ?? 0.7,
    history_encryption: settings.history_encryption ?? false,
    excluded_apps: settings.excluded_apps ?? [],
    duck_media: settings.duck_media ?? false,
    follow_default_mic: settings.follow_default_mic ?? true,
    voice_commands_enabled: settings.voice_commands_enabled ?? false,
    min_dictation_ms: settings.min_dictation_ms ?? 300,
    power_policy: settings.power_policy ?? { unload_on_battery: false, battery_preset: null },
    hud_contrast: settings.hud_contrast ?? "standard",
    paste_delay_ms: settings.paste_delay_ms ?? 30,
    inject_method: settings.inject_method ?? "paste",
    append_space: settings.append_space ?? false,
    capitalize_first: settings.capitalize_first ?? true,
    meeting_mode: settings.meeting_mode ?? false,
    meeting_monitor_device_id: settings.meeting_monitor_device_id ?? null,
    hotkeys: {
      dictation: settings.hotkey,
      command: settings.hotkey?.includes("Win") ? "Ctrl+Win+Alt" : "Ctrl+Alt+Space",
      assistant: null,
      note: null,
      cancel: "Escape",
      ...settings.hotkeys,
    },
    output_action: settings.output_action ?? { type: "paste" },
    send_key: settings.send_key ?? "enter",
    modes: settings.modes ?? defaultModes(),
    default_mode_id: settings.default_mode_id ?? "dictation",
    mode_triggers_enabled: settings.mode_triggers_enabled ?? true,
    snippets: settings.snippets ?? [],
    audio_retention: settings.audio_retention ?? "disabled",
    cleanup_level: inferCleanupLevel(settings),
    intelligence_tier,
    flow_model,
    preset: settings.preset ?? "auto",
    asr: { ...DEFAULT_ASR_SETTINGS, ...(settings.asr ?? {}) },
    refinement: {
      ...DEFAULT_REFINEMENT_SETTINGS,
      ...(settings.refinement ?? {}),
      // The tier owns this choice; the section follows. Without this the two
      // can disagree and the UI shows one model while another is loaded.
      model: flow_model,
    },
    streaming: { ...DEFAULT_STREAMING_SETTINGS, ...(settings.streaming ?? {}) },
    memory_policy: { ...DEFAULT_MEMORY_POLICY, ...(settings.memory_policy ?? {}) },
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

export interface FileProgress {
  job_id: string;
  done_s: number;
  total_s: number;
  finished: boolean;
  error: string | null;
  history_id: string | null;
}

export interface UsageStats {
  total_words: number;
  total_characters: number;
  dictations: number;
  minutes_spoken: number;
  time_saved_minutes: number;
  streak_days: number;
  per_app: {
    process: string;
    application_name: string;
    words: number;
    dictations: number;
    minutes_spoken: number;
  }[];
  per_day: { date: string; words: number; dictations: number; minutes_spoken: number }[];
}
