import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import {
  UsageStats,
  FileProgress,
  AppState,
  Mode,
  SessionIntent,
  AppSettings,
  AudioDevice,
  MicrophoneHealth,
  RecognitionTest,
  HistoryEntry,
  HistoryQuery,
  HistoryPage,
  RuntimeEntry,
  DictionaryTerm,
  CustomReplacement,
  Capabilities,
  DEFAULT_ASR_SETTINGS,
  DEFAULT_MEMORY_POLICY,
  DEFAULT_REFINEMENT_SETTINGS,
  DEFAULT_STREAMING_SETTINGS,
  ModelManifest,
  Preset,
  ProfileOverrides,
  ResolvedProfile,
  RuntimePlan,
  IntelligenceTierState,
  LatencyMetrics,
  LatencyPercentiles,
  LatencyReport,
  FullBenchmarkReport,
  SystemMetrics,
  ModelStatus,
  FlowStatus,
  PlatformInfo,
  ApiStatus,
  CalibrationStatus,
  normalizeSettings,
} from "../types";

/**
 * Zero-valued metrics for the browser-dev fallback.
 *
 * Deliberately a factory rather than a shared constant: callers render these
 * and a shared mutable object would let one view's edits leak into another.
 */
export function emptyLatencyMetrics(): LatencyMetrics {
  return {
    hotkey_to_recording_ms: 0,
    recording_to_first_audio_ms: 0,
    audio_to_first_partial_ms: 0,
    speech_end_to_final_ms: 0,
    final_to_injection_ms: 0,
    llm_startup_ms: 0,
    formatting_ms: 0,
    rewrite_ms: 0,
    rewrite_applied: false,
    release_to_inserted_ms: 0,
    total_duration_ms: 0,
    audio_duration_ms: 0,
    rtf: 0,
    segments: [],
    last_updated: new Date().toISOString(),
  };
}

/**
 * Browser-dev fallback. Every figure is zero and `gpu_present` is false, so a
 * caller can never mistake the fallback for a machine that has a usable GPU.
 */
export function emptySystemMetrics(): SystemMetrics {
  return {
    cpu_usage_pct: 0,
    app_ram_mb: 0,
    model_ram_mb: 0,
    total_ram_mb: 0,
    used_ram_mb: 0,
    available_ram_mb: 0,
    vram_mb: 0,
    total_vram_mb: 0,
    used_vram_mb: 0,
    free_vram_mb: 0,
    gpu_name: "CPU",
    gpu_vendor: "unknown",
    gpu_present: false,
    cuda_available: false,
    vulkan_available: false,
    cpu_model: "unknown",
    physical_cores: 0,
    logical_cores: 0,
    asr_ram_mb: 0,
    refinement_ram_mb: 0,
    model_loaded: false,
    backend_name: "CPU",
    os_name: "web",
    session: "unknown",
  };
}

export function emptyCapabilities(): Capabilities {
  return {
    gpus: [],
    cuda: {
      nvidia_gpu_present: false,
      driver_present: false,
      driver_cuda_version: null,
      torch_cuda_available: false,
      torch_cuda_version: null,
    },
    vulkan: { available: false, api_version: null, devices: [] },
    cpu: { model: "unknown", physical_cores: null, logical_cores: 0, load_pct: 0 },
    ram: {
      total_mb: 0,
      used_mb: 0,
      available_mb: 0,
      app_mb: 0,
      asr_mb: 0,
      refinement_mb: 0,
    },
    os_name: "web",
    probed_at: new Date().toISOString(),
  };
}

export function emptyLatencyPercentiles(): LatencyPercentiles {
  return {
    samples: 0,
    hotkey_to_recording_p50_ms: 0,
    hotkey_to_recording_p95_ms: 0,
    speech_end_to_final_p50_ms: 0,
    speech_end_to_final_p95_ms: 0,
    rewrite_p50_ms: 0,
    rewrite_p95_ms: 0,
    release_to_inserted_p50_ms: 0,
    release_to_inserted_p95_ms: 0,
    rtf_p50: 0,
    rtf_p95: 0,
    llm_applied_rate: 0,
  };
}

// Check if running inside Tauri
export const isTauri = (): boolean => {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
};

// Safe invoke wrapper with fallback for web dev
async function safeInvoke<T>(
  cmd: string,
  args?: Record<string, unknown>,
  fallback?: T,
): Promise<T> {
  if (isTauri()) {
    try {
      return await invoke<T>(cmd, args);
    } catch (err) {
      console.error(`Tauri command '${cmd}' failed:`, err);
      throw err;
    }
  }
  if (fallback !== undefined) {
    return fallback;
  }
  throw new Error(`Tauri environment not detected and no fallback provided for '${cmd}'`);
}

// Safe event listener wrapper
export async function safeListen<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<UnlistenFn> {
  if (isTauri()) {
    return await listen<T>(event, (e) => handler(e.payload));
  }
  console.log(`[Web Dev Mock] Subscribed to event: ${event}`);
  return () => {
    console.log(`[Web Dev Mock] Unsubscribed from event: ${event}`);
  };
}

const DEFAULT_SETTINGS: AppSettings = normalizeSettings({
  hotkey: "Shift+Win",
  push_to_talk: true,
  auto_stop_silence_ms: 1500,
  max_duration_sec: 60,
  language: "auto",
  auto_detect_language: true,
  microphone_device_id: null,
  input_gain: 1.0,
  vad_sensitivity: 0.5,
  processing_mode: "smart",
  dictation_mode: "normal",
  preset: "auto",
  asr: DEFAULT_ASR_SETTINGS,
  refinement: DEFAULT_REFINEMENT_SETTINGS,
  streaming: DEFAULT_STREAMING_SETTINGS,
  memory_policy: DEFAULT_MEMORY_POLICY,
  history_retention: "30_days",
  audio_retention: "disabled",
  overlay_position: "bottom_center",
  overlay_theme: "dark",
  app_theme: "system",
  accent_color: "sky",
  hud_scale: "standard",
  waveform_style: "bars",
  reduce_motion: false,
  ui_font_scale: "normal",
  developer_mode: false,
  cleanup_level: "light",
  intelligence_tier: "smart_flow",
  style: "neutral",
  auto_style_from_app: true,
  flow_model: "qwen3.5-0.8b",
  active_profile: "Default",
  launch_at_startup: false,
  start_minimized: false,
  offline_mode: true,
  spoken_punctuation_enabled: true,
  filler_removal_enabled: true,
  clipboard_restore_enabled: true,
  dictionary_terms: [],
  custom_replacements: [],
  api_enabled: false,
  api_bind: "lan",
  api_port: 7840,
  api_inject_default: false,
});

let previewSettings = structuredClone(DEFAULT_SETTINGS);

export const api = {
  // App State
  getAppState: () => safeInvoke<AppState>("get_app_state", undefined, "READY"),

  // Settings
  getSettings: () =>
    safeInvoke<AppSettings>("get_settings", undefined, structuredClone(previewSettings)),

  // Rust command argument is named `settings`.
  updateSettings: async (settings: Partial<AppSettings>) => {
    if (isTauri()) return safeInvoke<AppSettings>("update_settings", { settings });
    previewSettings = normalizeSettings({
      ...previewSettings,
      ...settings,
      hotkey: settings.hotkeys?.dictation ?? settings.hotkey ?? previewSettings.hotkey,
      hotkeys: {
        ...previewSettings.hotkeys!,
        ...settings.hotkeys,
        ...(settings.hotkey ? { dictation: settings.hotkey } : {}),
      },
      asr: { ...previewSettings.asr, ...settings.asr },
      refinement: { ...previewSettings.refinement, ...settings.refinement },
    });
    return structuredClone(previewSettings);
  },

  // Audio & Devices
  getAudioDevices: () =>
    safeInvoke<AudioDevice[]>("get_audio_devices", undefined, [
      {
        id: "default",
        name: "Default Microphone (Realtek(R) Audio)",
        is_default: true,
        sample_rate: 48000,
        channels: 2,
      },
    ]),

  setAudioDevice: (deviceId: string) => safeInvoke<void>("set_audio_device", { deviceId }),

  // Recording Control
  startRecording: (intent?: SessionIntent, modeId?: string) =>
    safeInvoke<void>("start_recording", { intent, modeId }),
  startMeeting: () => safeInvoke<void>("start_meeting"),
  summarizeHistory: (id: string) => safeInvoke<string>("summarize_history", { id }),
  executeAssistantTool: (response: string) =>
    safeInvoke<string>("execute_assistant_tool", { response }),
  stopRecording: () => safeInvoke<string>("stop_recording"),
  cancelRecording: () => safeInvoke<void>("cancel_recording"),

  // Manual Text Injection & Testing
  injectText: async (text: string) => {
    if (isTauri()) return safeInvoke<boolean>("inject_text", { text });
    await navigator.clipboard.writeText(text);
    return false;
  },
  testMicrophoneLevel: () => safeInvoke<number>("get_current_audio_level", undefined, 0.0),
  testMicrophone: () => safeInvoke<MicrophoneHealth>("test_microphone"),
  testRecognition: () => safeInvoke<RecognitionTest>("test_recognition"),
  getAsrProgress: () =>
    safeInvoke<{ completed: number; total: number }>("get_asr_progress", undefined, {
      completed: 0,
      total: 0,
    }),
  retryClipboardRestore: () => safeInvoke<boolean>("retry_clipboard_restore"),

  getStats: () =>
    safeInvoke<UsageStats>("get_stats", undefined, {
      total_words: 0,
      total_characters: 0,
      dictations: 0,
      minutes_spoken: 0,
      time_saved_minutes: 0,
      streak_days: 0,
      per_app: [],
      per_day: [],
    }),
  repasteLast: () => safeInvoke<boolean>("repaste_last"),
  getAppVersion: () => safeInvoke<string>("get_app_version", undefined, "0.3.0"),
  openReleases: () => safeInvoke<void>("open_releases"),
  getNetworkJournal: () =>
    safeInvoke<{ timestamp: string; host: string; bytes: number }[]>(
      "get_network_journal",
      { limit: 100 },
      [],
    ),
  exportConfig: (path: string) => safeInvoke<string>("export_config", { path }),
  importConfig: (path: string, replace: boolean) =>
    safeInvoke<AppSettings>("import_config", { path, replace }),
  importModelFile: (path: string) =>
    safeInvoke<{ id: string; path: string; bytes: number }>("import_model_file", { path }),
  getAutomationTokenStatus: () =>
    safeInvoke<{ id: string; name: string } | null>("get_automation_token_status", undefined, null),
  createAutomationToken: () =>
    safeInvoke<{ token: string; device: { id: string; name: string } }>("create_automation_token"),
  revokeAutomationToken: () => safeInvoke<boolean>("revoke_automation_token"),
  dismissAssistant: () => safeInvoke<void>("dismiss_assistant"),
  selectAudioFile: () => safeInvoke<string | null>("select_audio_file", undefined, null),
  transcribeFile: (path: string, allowLarge: boolean) =>
    safeInvoke<{ job_id: string }>("transcribe_file", { path, allowLarge }),
  getFileJob: (jobId: string) => safeInvoke<FileProgress>("get_file_job", { jobId }),
  cancelFileTranscription: (jobId: string) =>
    safeInvoke<void>("cancel_file_transcription", { jobId }),
  exportFileTranscript: (id: string, format: string) =>
    safeInvoke<string>("export_file_transcript", { id, format }),
  addNote: (text: string) => safeInvoke<HistoryEntry>("add_note", { text }),
  exportNotes: () => safeInvoke<string>("export_notes"),
  updateHistoryMetadata: (id: string, pinned: boolean, tags: string) =>
    safeInvoke<void>("update_history_metadata", { id, pinned, tags }),
  editHistoryTranscript: (id: string, text: string) =>
    safeInvoke<HistoryEntry>("edit_history_transcript", { id, text }),
  // History CRUD
  queryHistory: (query: HistoryQuery) =>
    safeInvoke<HistoryPage>(
      "query_history",
      { query },
      { entries: [], total: 0, next_offset: null, recovery_notice: null },
    ),
  exportHistory: (query: HistoryQuery) => safeInvoke<string>("export_history", { query }),
  getRuntimeInventory: () => safeInvoke<RuntimeEntry[]>("get_runtime_inventory", undefined, []),
  rollbackRuntime: () => safeInvoke<RuntimeEntry[]>("rollback_runtime"),
  repairRuntime: () => safeInvoke<void>("repair_runtime"),
  getHistory: (limit: number = 50, offset: number = 0) =>
    safeInvoke<HistoryEntry[]>("get_history", { limit, offset }, []),

  searchHistory: (query: string) => safeInvoke<HistoryEntry[]>("search_history", { query }, []),

  deleteHistoryItem: (id: string) => safeInvoke<boolean>("delete_history_item", { id }, true),

  // Returns the updated entry, or null when the id is gone.
  undoHistoryAiEdit: (id: string) =>
    safeInvoke<HistoryEntry | null>("undo_history_ai_edit", { id }, null),
  retryHistoryTranscript: (
    id: string,
    options?: { tier?: string; mode?: string; language?: string },
  ) => safeInvoke<HistoryEntry | null>("retry_history_transcript", { id, options }, null),
  extractHistoryAudio: (id: string) => safeInvoke<string>("extract_history_audio", { id }),

  clearTodayHistory: () => safeInvoke<number>("clear_today_history", undefined, 0),
  clearAllHistory: () => safeInvoke<number>("clear_all_history", undefined, 0),

  // Custom Dictionary & Replacements
  getDictionaryTerms: () => safeInvoke<DictionaryTerm[]>("get_dictionary_terms", undefined, []),

  saveDictionaryTerm: (term: Omit<DictionaryTerm, "id"> & { id?: string }) =>
    safeInvoke<DictionaryTerm>("save_dictionary_term", { term }),

  deleteDictionaryTerm: (id: string) => safeInvoke<boolean>("delete_dictionary_term", { id }, true),

  getCustomReplacements: () =>
    safeInvoke<CustomReplacement[]>("get_custom_replacements", undefined, []),

  saveCustomReplacement: (replacement: Omit<CustomReplacement, "id"> & { id?: string }) =>
    safeInvoke<CustomReplacement>("save_custom_replacement", { replacement }),

  deleteCustomReplacement: (id: string) =>
    safeInvoke<boolean>("delete_custom_replacement", { id }, true),

  // Model Management
  getModelStatus: () =>
    safeInvoke<ModelStatus>("get_model_status", undefined, {
      installed: false,
      loaded: false,
      version: "0.6B-v1",
      name: "Qwen/Qwen3-ASR-0.6B",
      size_bytes: 1880000000,
      download_progress_pct: 100,
      download_speed_mbps: 0,
      backend: "CPU",
      is_downloading: false,
      is_loading: false,
      error: null,
    }),

  installModel: (modelSize: string = "0.6b") => safeInvoke<void>("install_model", { modelSize }),

  removeModel: (modelSize: string = "0.6b") => safeInvoke<void>("remove_model", { modelSize }),

  reloadModel: () => safeInvoke<void>("reload_model"),

  // Metrics & Developer Diagnostics
  getLatencyMetrics: () =>
    safeInvoke<LatencyMetrics>("get_latency_metrics", undefined, emptyLatencyMetrics()),

  getLatencyReport: () =>
    safeInvoke<LatencyReport>("get_latency_report", undefined, {
      last: emptyLatencyMetrics(),
      percentiles: emptyLatencyPercentiles(),
      recent: [],
    }),

  resetLatencyHistory: () =>
    safeInvoke<LatencyReport>("reset_latency_history", undefined, {
      last: emptyLatencyMetrics(),
      percentiles: emptyLatencyPercentiles(),
      recent: [],
    }),

  getSystemMetrics: () =>
    safeInvoke<SystemMetrics>("get_system_metrics", undefined, emptySystemMetrics()),

  getBenchmarkReport: () =>
    safeInvoke<FullBenchmarkReport>("get_benchmark_report", undefined, {
      timestamp: new Date().toISOString(),
      hardware_summary: "Local Engine",
    }),

  runSystemBenchmark: () =>
    safeInvoke<FullBenchmarkReport | null>("run_system_benchmark", undefined, null),

  getCapabilities: () =>
    safeInvoke<Capabilities>("get_capabilities", undefined, emptyCapabilities()),

  /**
   * Force a fresh hardware probe. Call this before showing a load decision;
   * a cached free-VRAM reading is worse than no reading.
   */
  refreshCapabilities: () =>
    safeInvoke<Capabilities>("refresh_capabilities", undefined, emptyCapabilities()),

  getModelManifests: () => safeInvoke<ModelManifest[]>("get_model_manifests", undefined, []),

  /**
   * Resolve a configuration for the current hardware without applying it, so a
   * preset can be previewed with its reasons before the user commits.
   */
  previewProfile: (preset: Preset, overrides?: ProfileOverrides) =>
    safeInvoke<ResolvedProfile | null>("preview_profile", { preset, overrides }, null),
  getRuntimePlan: () => safeInvoke<RuntimePlan | null>("get_runtime_plan", undefined, null),

  openLogsFolder: () => safeInvoke<void>("open_logs_folder"),
  getDiagnosticsReport: () =>
    safeInvoke<string>(
      "get_diagnostics_report",
      undefined,
      "Diagnostic report unavailable in web preview.",
    ),
  // Convenience: fetch + write to clipboard in one call.
  copyDiagnostics: async (): Promise<boolean> => {
    try {
      const report = await api.getDiagnosticsReport();
      await navigator.clipboard.writeText(report);
      return true;
    } catch (e) {
      console.error("copyDiagnostics failed:", e);
      return false;
    }
  },
  getPlatformInfo: () =>
    safeInvoke<PlatformInfo>("get_platform_info", undefined, {
      os: "web",
      session: "unknown",
      default_hotkey: "Shift+Win",
      data_dir: "",
      logs_dir: "",
      hotkey_error: null,
      injection_notes: "Web preview cannot inject text.",
    }),

  getApiStatus: () =>
    safeInvoke<ApiStatus>("get_api_status", undefined, {
      enabled: false,
      running: false,
      bind: "lan",
      port: 7840,
      listen_addrs: ["127.0.0.1"],
      pairing_code: null,
      pairing_expires_in_sec: null,
      qr_svg: null,
      pair_uri: null,
      devices: [],
      warning: "LAN API is only available in the desktop app.",
    }),
  rotatePairingCode: () => safeInvoke<ApiStatus>("rotate_pairing_code"),
  listApiDevices: () => safeInvoke<ApiStatus["devices"]>("list_api_devices", undefined, []),
  getCalibrationStatus: () =>
    safeInvoke<CalibrationStatus>("get_calibration_status", undefined, {
      running: false,
      phase: "idle",
      candidates: [],
      winner_id: null,
      error: null,
      language: "en",
      reference: "",
    }),
  runCalibration: (args: { reference: string; language: string; seconds: number }) =>
    safeInvoke<CalibrationStatus>("run_calibration", args),
  cancelCalibration: () => safeInvoke<void>("cancel_calibration"),
  applyCalibration: (id: string) => safeInvoke<boolean>("apply_calibration", { id }),
  revokeApiDevice: (id: string) => safeInvoke<boolean>("revoke_api_device", { id }, true),
  setApiDevicePermissions: (
    id: string,
    permissions: { stream: boolean; history: boolean; injection: boolean },
  ) => safeInvoke<boolean>("set_api_device_permissions", { id, permissions }, true),
  getFlowStatus: () =>
    safeInvoke<FlowStatus>("get_flow_status", undefined, {
      active_tier: "smart_flow",
      active_model: "qwen3.5-0.8b",
      ready: false,
      installed: false,
      runtime_installed: false,
      backend: "none",
      is_loading: false,
      is_downloading: false,
      download_progress_pct: 0,
    }),
  previewCleanup: (
    text: string,
    tier: AppSettings["intelligence_tier"] = "smart_flow",
    style?: string,
    mode?: Mode,
  ) =>
    safeInvoke<{
      text: string;
      latency_ms: number;
      tier_used: AppSettings["intelligence_tier"];
      model_used: string;
    }>(
      "preview_tier_cleanup",
      { text, tier, style, mode },
      {
        text,
        latency_ms: 0,
        tier_used: tier,
        model_used:
          tier === "deep_context" ? "qwen3.5-2b" : tier === "smart_flow" ? "qwen3.5-0.8b" : "none",
      },
    ),
  getIntelligenceStatus: () =>
    safeInvoke<FlowStatus>("get_intelligence_status", undefined, {
      active_tier: "smart_flow",
      active_model: "qwen3.5-0.8b",
      ready: false,
      installed: false,
      runtime_installed: false,
      backend: "none",
      is_loading: false,
      is_downloading: false,
      download_progress_pct: 0,
    }),
  getIntelligenceTiers: () =>
    safeInvoke<IntelligenceTierState[]>("get_intelligence_tiers", undefined, []),
  installIntelligenceModel: (tier: AppSettings["intelligence_tier"]) =>
    safeInvoke<void>("install_intelligence_model", { tier }),
  removeIntelligenceModel: (tier: AppSettings["intelligence_tier"]) =>
    safeInvoke<void>("remove_intelligence_model", { tier }),
  installLlamaRuntime: (computeBackend: AppSettings["refinement"]["device"]) =>
    safeInvoke<void>("install_llama_runtime", { computeBackend }),
  removeLlamaRuntime: () => safeInvoke<void>("remove_llama_runtime"),
  setIntelligenceTier: (tier: AppSettings["intelligence_tier"]) =>
    safeInvoke<AppSettings>("set_intelligence_tier", { tier }),
  /**
   * Whether the LLM polish pass is currently on.
   *
   * Defaults to `true` when the backend cannot be reached, matching the shipped
   * default tier, so the HUD badge does not claim Fast mode on a transient IPC
   * failure.
   */
  getPolishEnabled: () => safeInvoke<boolean>("get_polish_enabled", undefined, true),
  /**
   * Flip between Fast (ASR only) and Polished (ASR + LLM).
   *
   * Persists, and takes effect on the next dictation: the stop path re-reads
   * settings each time, so no reload is needed. Enabling also starts and warms
   * the runtime so the first polished dictation is not the one that pays for it.
   */
  setPolishEnabled: (enabled: boolean) =>
    safeInvoke<AppSettings>("set_polish_enabled", { enabled }),
  previewTierCleanup: (text: string, tier: AppSettings["intelligence_tier"], style?: string) =>
    api.previewCleanup(text, tier, style),
  // Returns pre-edit text for recovery; never pastes into the foreground app.
  undoLastAiEdit: () => safeInvoke<string>("undo_last_ai_edit", undefined, ""),

  // App lifecycle
  quit: () => safeInvoke<void>("quit_app", undefined, undefined as unknown as void),
};

export const tauriApi = api;
