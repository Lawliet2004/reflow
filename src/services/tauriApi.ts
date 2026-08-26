import { invoke } from "@tauri-apps/api/core";
import { listen, UnlistenFn } from "@tauri-apps/api/event";
import {
  AppState,
  AppSettings,
  AudioDevice,
  HistoryEntry,
  DictionaryTerm,
  CustomReplacement,
  IntelligenceTierState,
  LatencyMetrics,
  SystemMetrics,
  ModelStatus,
  FlowStatus,
  StreamingTranscriptPayload,
  PlatformInfo,
  ApiStatus,
  RuntimeDownloadEvent,
} from "../types";

// Check if running inside Tauri
export const isTauri = (): boolean => {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
};

// Safe invoke wrapper with fallback for web dev
async function safeInvoke<T>(cmd: string, args?: Record<string, unknown>, fallback?: T): Promise<T> {
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
  handler: (payload: T) => void
): Promise<UnlistenFn> {
  if (isTauri()) {
    return await listen<T>(event, (e) => handler(e.payload));
  }
  console.log(`[Web Dev Mock] Subscribed to event: ${event}`);
  return () => {
    console.log(`[Web Dev Mock] Unsubscribed from event: ${event}`);
  };
}

const DEFAULT_SETTINGS: AppSettings = {
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
  compute_backend: "auto",
  asr_model: "0.6b",
  asr_precision: "auto",
  flow_n_gpu_layers: -1,
  keep_model_loaded: true,
  history_retention: "30_days",
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
  flow_model: "lfm2.5-1.2b",
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
  api_mdns: true,
  api_inject_default: false,
};

export const api = {
  // App State
  getAppState: () => safeInvoke<AppState>("get_app_state", undefined, "READY"),

  // Settings
  getSettings: () => safeInvoke<AppSettings>("get_settings", undefined, DEFAULT_SETTINGS),

  // Rust command argument is named `settings`.
  updateSettings: (settings: Partial<AppSettings>) =>
    safeInvoke<AppSettings>("update_settings", { settings }),

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

  setAudioDevice: (deviceId: string) =>
    safeInvoke<void>("set_audio_device", { deviceId }),

  // Recording Control
  startRecording: () => safeInvoke<void>("start_recording"),
  stopRecording: () => safeInvoke<string>("stop_recording", undefined, "Simulated transcript output."),
  cancelRecording: () => safeInvoke<void>("cancel_recording"),

  // Manual Text Injection & Testing
  injectText: (text: string) => safeInvoke<boolean>("inject_text", { text }, true),
  testMicrophoneLevel: () => safeInvoke<number>("get_current_audio_level", undefined, 0.0),

  // History CRUD
  getHistory: (limit: number = 50, offset: number = 0) =>
    safeInvoke<HistoryEntry[]>("get_history", { limit, offset }, []),

  searchHistory: (query: string) =>
    safeInvoke<HistoryEntry[]>("search_history", { query }, []),

  deleteHistoryItem: (id: string) =>
    safeInvoke<boolean>("delete_history_item", { id }, true),

  clearTodayHistory: () => safeInvoke<number>("clear_today_history", undefined, 0),
  clearAllHistory: () => safeInvoke<number>("clear_all_history", undefined, 0),

  // Custom Dictionary & Replacements
  getDictionaryTerms: () =>
    safeInvoke<DictionaryTerm[]>("get_dictionary_terms", undefined, []),

  saveDictionaryTerm: (term: Omit<DictionaryTerm, "id"> & { id?: string }) =>
    safeInvoke<DictionaryTerm>("save_dictionary_term", { term }),

  deleteDictionaryTerm: (id: string) =>
    safeInvoke<boolean>("delete_dictionary_term", { id }, true),

  getCustomReplacements: () =>
    safeInvoke<CustomReplacement[]>("get_custom_replacements", undefined, []),

  saveCustomReplacement: (replacement: Omit<CustomReplacement, "id"> & { id?: string }) =>
    safeInvoke<CustomReplacement>("save_custom_replacement", { replacement }),

  deleteCustomReplacement: (id: string) =>
    safeInvoke<boolean>("delete_custom_replacement", { id }, true),

  // Model Management
  getModelStatus: () =>
    safeInvoke<ModelStatus>("get_model_status", undefined, {
      installed: true,
      loaded: true,
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

  installModel: (modelSize: string = "0.6b") =>
    safeInvoke<void>("install_model", { modelSize }),

  removeModel: (modelSize: string = "0.6b") =>
    safeInvoke<void>("remove_model", { modelSize }),

  reloadModel: () => safeInvoke<void>("reload_model"),

  // Metrics & Developer Diagnostics
  getLatencyMetrics: () =>
    safeInvoke<LatencyMetrics>("get_latency_metrics", undefined, {
      hotkey_to_recording_ms: 0,
      recording_to_first_audio_ms: 0,
      audio_to_first_partial_ms: 0,
      speech_end_to_final_ms: 0,
      final_to_injection_ms: 0,
      total_duration_ms: 0,
      rewrite_ms: 0,
      last_updated: new Date().toISOString(),
    }),

  getSystemMetrics: () =>
    safeInvoke<SystemMetrics>("get_system_metrics", undefined, {
      cpu_usage_pct: 0,
      app_ram_mb: 0,
      model_ram_mb: 0,
      total_ram_mb: 0,
      vram_mb: 0,
      gpu_name: "CPU",
      model_loaded: false,
      backend_name: "CPU",
      os_name: "web",
      session: "unknown",
    }),

  openLogsFolder: () => safeInvoke<void>("open_logs_folder"),
  getDiagnosticsReport: () =>
    safeInvoke<string>("get_diagnostics_report", undefined, "Diagnostic report unavailable in web preview."),
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
  revokeApiDevice: (id: string) => safeInvoke<boolean>("revoke_api_device", { id }, true),
  getFlowStatus: () =>
    safeInvoke<FlowStatus>("get_flow_status", undefined, {
      active_tier: "smart_flow",
      active_model: "lfm2.5-1.2b",
      ready: false,
      installed: false,
      runtime_installed: false,
      backend: "none",
      is_loading: false,
      is_downloading: false,
      download_progress_pct: 0,
    }),
  previewCleanup: (text: string, tier: AppSettings["intelligence_tier"] = "smart_flow", style?: string) =>
    safeInvoke<{ text: string; latency_ms: number; tier_used: AppSettings["intelligence_tier"]; model_used: string }>("preview_tier_cleanup", { text, tier, style }, {
      text,
      latency_ms: 0,
      tier_used: tier,
      model_used: tier === "deep_context" ? "qwen3.5-2b" : tier === "smart_flow" ? "lfm2.5-1.2b" : "none",
    }),
  getIntelligenceStatus: () => safeInvoke<FlowStatus>("get_intelligence_status", undefined, {
    active_tier: "smart_flow",
    active_model: "lfm2.5-1.2b",
    ready: false,
    installed: false,
    runtime_installed: false,
    backend: "none",
    is_loading: false,
    is_downloading: false,
    download_progress_pct: 0,
  }),
  getIntelligenceTiers: () => safeInvoke<IntelligenceTierState[]>("get_intelligence_tiers", undefined, []),
  installIntelligenceModel: (tier: AppSettings["intelligence_tier"]) => safeInvoke<void>("install_intelligence_model", { tier }),
  removeIntelligenceModel: (tier: AppSettings["intelligence_tier"]) => safeInvoke<void>("remove_intelligence_model", { tier }),
  installLlamaRuntime: (computeBackend: AppSettings["compute_backend"]) => safeInvoke<void>("install_llama_runtime", { computeBackend }),
  removeLlamaRuntime: () => safeInvoke<void>("remove_llama_runtime"),
  setIntelligenceTier: (tier: AppSettings["intelligence_tier"]) => safeInvoke<AppSettings>("set_intelligence_tier", { tier }),
  previewTierCleanup: (text: string, tier: AppSettings["intelligence_tier"], style?: string) => api.previewCleanup(text, tier, style),
  // Server-side: re-injects pre-LLM text from the latest history entry.
  undoLastAiEdit: () => safeInvoke<string>("undo_last_ai_edit", undefined, ""),

  // App lifecycle
  quit: () => safeInvoke<void>("quit_app", undefined, undefined as unknown as void),
};
