import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { api } from "../../services/tauriApi";
import { ModesPage } from "./ModesPage";

describe("Modes", () => {
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
