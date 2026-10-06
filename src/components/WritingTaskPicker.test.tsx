import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { api } from "../services/tauriApi";
import { WritingTaskPicker } from "./WritingTaskPicker";

describe("writing task picker", () => {
  it("explains natural cleanup and applies the task without changing ASR", async () => {
    const settings = { ...(await api.getSettings()), cleanup_level: "medium" as const };
    const save = vi.fn().mockResolvedValue(true);
    render(<WritingTaskPicker settings={settings} onUpdateSettings={save} />);
    expect(screen.getByRole("combobox", { name: "Writing task" })).toHaveValue("natural");
    fireEvent.change(screen.getByRole("combobox", { name: "Writing task" }), {
      target: { value: "developer" },
    });
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith(
        expect.objectContaining({ dictation_mode: "developer_prompt", cleanup_level: "medium" }),
      ),
    );
    expect(save.mock.calls[0][0]).not.toHaveProperty("asr");
  });

  it("reports failed saves and retains the previous selection", async () => {
    render(
      <WritingTaskPicker
        settings={{ ...(await api.getSettings()), cleanup_level: "medium" }}
        onUpdateSettings={vi.fn().mockResolvedValue(false)}
      />,
    );
    fireEvent.change(screen.getByRole("combobox", { name: "Writing task" }), {
      target: { value: "quick" },
    });
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save");
    expect(screen.getByRole("combobox", { name: "Writing task" })).toHaveValue("natural");
  });

  it("disables task changes during recording and shows mode override information", async () => {
    render(
      <WritingTaskPicker
        settings={{ ...(await api.getSettings()), default_mode_id: "coding" }}
        onUpdateSettings={vi.fn()}
        disabled
      />,
    );
    expect(screen.getByRole("combobox", { name: "Writing task" })).toBeDisabled();
    expect(screen.getByText(/currently overrides/)).toBeInTheDocument();
  });
});
