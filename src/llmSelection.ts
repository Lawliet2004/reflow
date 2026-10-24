import { AppSettings, FlowModel, flowModelForTier, polishEnabledFor } from "./types";

export const LLM_NAMES: Record<FlowModel, string> = {
  none: "No LLM · basic cleanup",
  "qwen3.5-4b": "Qwen3.5 4B",
  "minicpm5-2b": "MiniCPM5 2B",
  "gemma4-e4b": "Gemma 4 E4B",
  "qwen3.5-0.8b": "Qwen3.5 0.8B · lighter",
  "qwen3.5-2b": "Qwen3.5 2B · more capable",
};

type LlmPreferences = Partial<
  Pick<
    AppSettings,
    "preset" | "cleanup_level" | "intelligence_tier" | "flow_model" | "last_polish_model"
  >
>;

export function selectedLlm(settings: LlmPreferences): FlowModel {
  return settings.preset === "fast" || !polishEnabledFor(settings)
    ? "none"
    : settings.flow_model &&
        ["qwen3.5-4b", "minicpm5-2b", "gemma4-e4b"].includes(settings.flow_model)
      ? settings.flow_model
      : flowModelForTier(settings.intelligence_tier);
}

// Model selection is independent of polishing depth. Do not resend compute-device settings:
// that is an explicit runtime-install request, unrelated to picking an LLM.
export function llmSelectionPatch(
  settings: LlmPreferences,
  model: FlowModel,
): Partial<AppSettings> {
  const intelligence_tier =
    model === "none" ? "raw_verbatim" : model === "qwen3.5-2b" ? "deep_context" : "smart_flow";
  return {
    intelligence_tier,
    flow_model: model,
    ...(model !== "none" && settings.preset === "fast" ? { preset: "auto" as const } : {}),
    ...(model !== "none" && settings.cleanup_level === "raw"
      ? { cleanup_level: "light" as const }
      : {}),
    ...(model === "none" && ["medium", "high"].includes(settings.cleanup_level ?? "")
      ? { cleanup_level: "light" as const }
      : {}),
  };
}
