import { describe, expect, it } from "vitest";
import {
  normalizeSettings,
  inferCleanupLevel,
  flowModelForTier,
  tierForCleanupLevel,
  DEFAULT_ASR_SETTINGS,
  DEFAULT_REFINEMENT_SETTINGS,
  DEFAULT_STREAMING_SETTINGS,
  DEFAULT_MEMORY_POLICY,
  AppSettings,
  isModelLoading,
  ModelStatus,
} from "./index";

describe("normalizeSettings", () => {
  const minimalSettings = {
    hotkey: "ctrl+space",
    audio_device: "default",
    vad_sensitivity: "medium",
    language: "auto",
    dictation_mode: "normal",
    processing_mode: "smart",
    strip_filler_words: true,
    strip_stutters: true,
    auto_punctuate: true,
    smart_formatting: true,
    remove_trailing_period: false,
    paste_mode: "clipboard",
    history_retention: "30_days",
    history_save_audio: false,
  } as unknown as AppSettings;

  it("populates default nested sections when missing entirely", () => {
    const normalized = normalizeSettings(minimalSettings);

    expect(normalized.asr).toEqual(DEFAULT_ASR_SETTINGS);
    expect(normalized.refinement).toEqual({
      ...DEFAULT_REFINEMENT_SETTINGS,
      model: flowModelForTier(normalized.intelligence_tier),
    });
    expect(normalized.streaming).toEqual(DEFAULT_STREAMING_SETTINGS);
    expect(normalized.memory_policy).toEqual(DEFAULT_MEMORY_POLICY);
    expect(normalized.preset).toBe("auto");
    expect(normalized.audio_retention).toBe("disabled");
  });

  it("preserves partial nested configuration while backfilling missing fields", () => {
    const custom = {
      ...minimalSettings,
      asr: {
        model: "1.7b",
      } as unknown as typeof DEFAULT_ASR_SETTINGS,
      refinement: {
        device: "cpu",
        gpu_layers: 0,
      } as unknown as typeof DEFAULT_REFINEMENT_SETTINGS,
      streaming: {
        mode: "always_on",
      } as unknown as typeof DEFAULT_STREAMING_SETTINGS,
      memory_policy: {
        vram_reserve_mb: 512,
      } as unknown as typeof DEFAULT_MEMORY_POLICY,
    } as AppSettings;

    const normalized = normalizeSettings(custom);

    expect(normalized.asr).toEqual({
      runtime: "python",
      model: "1.7b",
      device: DEFAULT_ASR_SETTINGS.device,
      precision: DEFAULT_ASR_SETTINGS.precision,
      keep_loaded: DEFAULT_ASR_SETTINGS.keep_loaded,
    });
    expect(normalized.refinement.device).toBe("cpu");
    expect(normalized.refinement.gpu_layers).toBe(0);
    expect(normalized.refinement.context_size).toBe(DEFAULT_REFINEMENT_SETTINGS.context_size);
    expect(normalized.refinement.deadline_ms).toBe(DEFAULT_REFINEMENT_SETTINGS.deadline_ms);
    expect(normalized.streaming.mode).toBe("always_on");
    expect(normalized.streaming.silence_boundary_ms).toBe(
      DEFAULT_STREAMING_SETTINGS.silence_boundary_ms,
    );
    expect(normalized.memory_policy.vram_reserve_mb).toBe(512);
    expect(normalized.memory_policy.allow_gpu_refinement).toBe(false);
  });

  it("synchronizes refinement.model with intelligence_tier", () => {
    const rawNormalized = normalizeSettings({
      ...minimalSettings,
      intelligence_tier: "raw_verbatim",
      refinement: {
        ...DEFAULT_REFINEMENT_SETTINGS,
        model: "qwen3.5-0.8b",
      },
    });
    expect(rawNormalized.refinement.model).toBe("none");
    expect(rawNormalized.flow_model).toBe("none");

    const deepNormalized = normalizeSettings({
      ...minimalSettings,
      intelligence_tier: "deep_context",
    });
    expect(deepNormalized.refinement.model).toBe("qwen3.5-2b");
    expect(deepNormalized.flow_model).toBe("qwen3.5-2b");
  });

  it("infers cleanup_level and intelligence_tier when missing", () => {
    const normalized = normalizeSettings({
      ...minimalSettings,
      processing_mode: "raw",
    });
    expect(normalized.cleanup_level).toBe("raw");
    expect(normalized.intelligence_tier).toBe("raw_verbatim");
    expect(normalized.flow_model).toBe("none");
  });

  it("populates UI styling defaults when omitted", () => {
    const normalized = normalizeSettings(minimalSettings);
    expect(normalized.style).toBe("neutral");
    expect(normalized.auto_style_from_app).toBe(true);
    expect(normalized.developer_mode).toBe(false);
    expect(normalized.app_theme).toBe("system");
    expect(normalized.accent_color).toBe("sky");
    expect(normalized.hud_scale).toBe("standard");
    expect(normalized.waveform_style).toBe("bars");
    expect(normalized.reduce_motion).toBe(false);
    expect(normalized.ui_font_scale).toBe("normal");
    expect(normalized.overlay_theme).toBe("dark");
    expect(normalized.overlay_position).toBe("bottom_center");
  });
});

it("does not describe absent model weights as an active load", () => {
  const status = {
    installed: false,
    loaded: false,
    is_downloading: false,
    error: null,
  } as ModelStatus;
  expect(isModelLoading(status)).toBe(false);
  expect(isModelLoading({ ...status, installed: true })).toBe(true);
});

describe("cleanup and tier helpers", () => {
  it("infers cleanup levels from processing mode", () => {
    expect(inferCleanupLevel({ cleanup_level: "high", processing_mode: "raw" })).toBe("high");
    expect(inferCleanupLevel({ cleanup_level: undefined as any, processing_mode: "raw" })).toBe(
      "raw",
    );
    expect(inferCleanupLevel({ cleanup_level: undefined as any, processing_mode: "flow" })).toBe(
      "medium",
    );
    expect(inferCleanupLevel({ cleanup_level: undefined as any, processing_mode: "smart" })).toBe(
      "light",
    );
  });

  it("maps tier to flow model correctly", () => {
    expect(flowModelForTier("raw_verbatim")).toBe("none");
    expect(flowModelForTier("smart_flow")).toBe("qwen3.5-0.8b");
    expect(flowModelForTier("deep_context")).toBe("qwen3.5-2b");
  });

  it("maps cleanup level to tier correctly", () => {
    expect(tierForCleanupLevel("raw")).toBe("raw_verbatim");
    // `light` must NOT imply verbatim. It is a gentle Stage 1 cleanup, and
    // mapping it here silently disabled the LLM for anyone who chose it — the
    // same conflation that was removed from the Rust `tier_for_cleanup`.
    expect(tierForCleanupLevel("light")).toBe("smart_flow");
    expect(tierForCleanupLevel("medium")).toBe("smart_flow");
    expect(tierForCleanupLevel("high")).toBe("deep_context");
  });
});
