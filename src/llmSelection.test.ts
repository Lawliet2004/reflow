import { expect, it } from "vitest";
import { api } from "./services/tauriApi";
import { llmSelectionPatch, selectedLlm } from "./llmSelection";
import { normalizeSettings, polishEnabledFor } from "./types";

it("enables an explicitly chosen LLM even after Original or the Fast preset", async () => {
  const settings = {
    ...(await api.getSettings()),
    preset: "fast" as const,
    cleanup_level: "raw" as const,
    intelligence_tier: "raw_verbatim" as const,
  };
  const patch = llmSelectionPatch(settings, "qwen3.5-2b");
  const updated = normalizeSettings({ ...settings, ...patch });
  expect(updated.preset).toBe("auto");
  expect(updated.cleanup_level).toBe("light");
  expect(updated.refinement.model).toBe("qwen3.5-2b");
  expect(selectedLlm(updated)).toBe("qwen3.5-2b");
  expect(polishEnabledFor(updated)).toBe(true);
  expect(patch).not.toHaveProperty("refinement");
  expect(patch).not.toHaveProperty("asr");
});

it("turns AI off while preserving speech recognition and basic cleanup", async () => {
  const settings = {
    ...(await api.getSettings()),
    cleanup_level: "high" as const,
    intelligence_tier: "deep_context" as const,
  };
  const updated = normalizeSettings({ ...settings, ...llmSelectionPatch(settings, "none") });
  expect(selectedLlm(updated)).toBe("none");
  expect(updated.refinement.model).toBe("none");
  expect(polishEnabledFor(updated)).toBe(false);
  expect(updated.cleanup_level).toBe("light");
  expect(updated.asr).toEqual(settings.asr);
  expect(updated.memory_policy).toEqual(settings.memory_policy);
});
