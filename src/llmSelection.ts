import { AppSettings, FlowModel, flowModelForTier, polishEnabledFor } from "./types";

export const LLM_NAMES: Record<FlowModel, string> = {
  none: "No LLM · basic cleanup",
  "qwen3.5-0.8b": "Qwen3.5 0.8B · lighter",
  "qwen3.5-2b": "Qwen3.5 2B · more capable",
};

type LlmPreferences = Partial<Pick<AppSettings, "preset" | "cleanup_level" | "intelligence_tier">>;

export function selectedLlm(settings: LlmPreferences): FlowModel {
  return settings.preset === "fast" || !polishEnabledFor(settings)
    ? "none"
    : flowModelForTier(settings.intelligence_tier);
}

// The tier owns the backend's model choice. Do not resend compute-device settings:
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
