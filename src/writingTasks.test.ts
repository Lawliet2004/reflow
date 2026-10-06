import { describe, expect, it } from "vitest";
import { api } from "./services/tauriApi";
import { selectedWritingTask, writingTaskPatch } from "./writingTasks";
import { normalizeSettings } from "./types";
import { selectedLlm } from "./llmSelection";

describe("writing tasks", () => {
  it("replaces hidden Dictation instructions when a task is explicitly chosen", async () => {
    const original = await api.getSettings();
    const settings = {
      ...original,
      modes: (original.modes ?? []).map((mode) =>
        mode.id === "dictation"
          ? { ...mode, custom_instructions: "Summarize every dictation", translate_to: "French" }
          : mode,
      ),
    };
    expect(selectedWritingTask(settings)).toBe("custom");
    const patch = writingTaskPatch(settings, "natural");
    expect(patch.modes?.[0]).toEqual({
      ...settings.modes[0],
      custom_instructions: "",
      translate_to: null,
    });
    expect(patch.modes?.slice(1)).toEqual(settings.modes.slice(1));
    expect(selectedWritingTask({ ...settings, ...patch })).toBe("natural");
  });
  it("keeps quick cleanup without an LLM and preserves speech model settings", async () => {
    const settings = await api.getSettings();
    const next = normalizeSettings({ ...settings, ...writingTaskPatch(settings, "quick") });
    expect(selectedWritingTask(next)).toBe("quick");
    expect(selectedLlm(next)).toBe("none");
    expect(next.cleanup_level).toBe("light");
    expect(next.asr).toEqual(settings.asr);
  });

  it("activates natural cleanup from Original or the Fast preset without downloading", async () => {
    const settings = {
      ...(await api.getSettings()),
      preset: "fast" as const,
      default_mode_id: "coding",
    };
    const next = normalizeSettings({ ...settings, ...writingTaskPatch(settings, "natural") });
    expect(selectedWritingTask(next)).toBe("natural");
    expect(selectedLlm(next)).toBe("qwen3.5-0.8b");
    expect(next.preset).toBe("auto");
    expect(next.default_mode_id).toBe("dictation");
    expect(next.auto_style_from_app).toBe(false);
  });

  it("uses a developer-prompt context rather than literal Coding", async () => {
    const next = normalizeSettings({
      ...(await api.getSettings()),
      ...writingTaskPatch({}, "developer"),
    });
    expect(next.dictation_mode).toBe("developer_prompt");
    expect(next.cleanup_level).toBe("medium");
    expect(selectedLlm(next)).toBe("qwen3.5-2b");
    expect(selectedWritingTask({ ...next, intelligence_tier: "smart_flow" })).toBe("developer");
  });

  it("does not mislabel saved literal code settings or custom instructions", async () => {
    const settings = await api.getSettings();
    expect(
      selectedWritingTask({ ...settings, dictation_mode: "coding", cleanup_level: "light" }),
    ).toBe("custom");
    expect(selectedWritingTask({ ...settings, custom_instructions: "Translate this" })).toBe(
      "custom",
    );
    expect(selectedWritingTask({ ...settings, translate_to: "French" })).toBe("custom");
    expect(
      selectedWritingTask({
        ...settings,
        context: { selected_text: false, clipboard: true, window_title: false },
      }),
    ).toBe("custom");
  });
});
