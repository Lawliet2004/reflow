import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { api } from "../../services/tauriApi";
import { ModesPage } from "./ModesPage";
import { defaultModes } from "../../types";

describe("Modes", () => {
  it("shows an app-aware Dictation draft as custom until a task is chosen", async () => {
    render(
      <ModesPage
        settings={{ ...(await api.getSettings()), auto_style_from_app: true }}
        onUpdateSettings={vi.fn().mockResolvedValue(true)}
      />,
    );
    fireEvent.click(screen.getAllByRole("button", { name: "Edit" })[0]);
    expect(screen.getByRole("combobox", { name: "Writing task" })).toHaveValue("custom");
    fireEvent.change(screen.getByRole("combobox", { name: "Writing task" }), {
      target: { value: "natural" },
    });
    expect(screen.getByRole("combobox", { name: "Writing task" })).toHaveValue("natural");
  });
  it("applies the chosen task to Dictation without hidden translation or context overrides", async () => {
    const original = await api.getSettings();
    const settings = {
      ...original,
      preset: "fast" as const,
      auto_style_from_app: true,
      modes: (original.modes ?? defaultModes()).map((mode) =>
        mode.id === "dictation"
          ? {
              ...mode,
              translate_to: "French",
              context: { selected_text: false, clipboard: true, window_title: false },
            }
          : mode,
      ),
    };
    const save = vi.fn().mockResolvedValue(true);
    render(<ModesPage settings={settings} onUpdateSettings={save} />);
    fireEvent.click(screen.getAllByRole("button", { name: "Edit" })[0]);
    expect(screen.getByRole("combobox", { name: "Writing task" })).toHaveValue("custom");
    fireEvent.change(screen.getByRole("combobox", { name: "Writing task" }), {
      target: { value: "natural" },
    });
    expect(screen.getByRole("combobox", { name: "Translate output to" })).toHaveValue("");
    expect(screen.getByRole("switch", { name: "Read clipboard" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    fireEvent.click(screen.getByRole("button", { name: "Save mode" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith(
        expect.objectContaining({
          cleanup_level: "medium",
          style: "faithful",
          preset: "auto",
          auto_style_from_app: false,
          modes: expect.arrayContaining([
            expect.objectContaining({
              id: "dictation",
              translate_to: null,
              context: { selected_text: false, clipboard: false, window_title: false },
            }),
          ]),
        }),
      ),
    );
  });
  it("adds a preset with explicit context opt-in and previews its instructions", async () => {
    const settings = await api.getSettings();
    const save = vi.fn().mockResolvedValue(true);
    const preview = vi.spyOn(api, "previewCleanup").mockResolvedValue({
      text: "Ticket result",
      latency_ms: 1,
      tier_used: "smart_flow",
      model_used: "qwen3.5-0.8b",
    });
    render(<ModesPage settings={settings} onUpdateSettings={save} />);
    fireEvent.click(screen.getByRole("button", { name: "Jira ticket" }));
    expect(
      (screen.getByRole("textbox", { name: "Custom instructions" }) as HTMLTextAreaElement).value,
    ).toContain("Jira ticket");
    fireEvent.click(screen.getByRole("button", { name: "Preview" }));
    await screen.findByText("Ticket result");
    expect(preview.mock.calls[0][3]?.context).toEqual({
      selected_text: false,
      clipboard: false,
      window_title: false,
    });
    fireEvent.click(screen.getByRole("button", { name: "Save mode" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith({
        modes: expect.arrayContaining([
          expect.objectContaining({
            name: "Jira ticket",
            custom_instructions: expect.stringContaining("Jira ticket"),
          }),
        ]),
      }),
    );
  });
});
