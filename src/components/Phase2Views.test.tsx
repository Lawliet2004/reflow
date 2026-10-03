import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { api } from "../services/tauriApi";
import { FileTranscription } from "./FileTranscription";
import { NotesView } from "./NotesView";
import { DictionaryPage } from "./settings/DictionaryPage";

it("submits the large-file opt-in explicitly and cancels its own job", async () => {
  const start = vi.spyOn(api, "transcribeFile").mockResolvedValue({ job_id: "test-job" });
  vi.spyOn(api, "getFileJob").mockResolvedValue({
    job_id: "test-job",
    done_s: 10,
    total_s: 100,
    finished: false,
    error: null,
    history_id: null,
  });
  const cancel = vi.spyOn(api, "cancelFileTranscription").mockResolvedValue();
  render(<FileTranscription onOpenHistory={vi.fn()} />);
  fireEvent.change(screen.getByRole("textbox", { name: "Audio file path" }), {
    target: { value: "C:/audio.wav" },
  });
  fireEvent.click(screen.getByRole("checkbox"));
  fireEvent.click(screen.getByRole("button", { name: "Transcribe file" }));
  await waitFor(() => expect(start).toHaveBeenCalledWith("C:/audio.wav", true));
  fireEvent.click(await screen.findByRole("button", { name: "Cancel import" }));
  await waitFor(() => expect(cancel).toHaveBeenCalledWith("test-job"));
});

it("queries only notes and preserves a draft if saving fails", async () => {
  const query = vi
    .spyOn(api, "queryHistory")
    .mockResolvedValue({ entries: [], total: 0, next_offset: null, recovery_notice: null });
  vi.spyOn(api, "addNote").mockRejectedValue(new Error("Disk full"));
  render(
    <NotesView
      settings={await api.getSettings()}
      onUpdateSettings={vi.fn().mockResolvedValue(true)}
    />,
  );
  await waitFor(() =>
    expect(query).toHaveBeenCalledWith(expect.objectContaining({ kind: "note" })),
  );
  fireEvent.change(screen.getByRole("textbox", { name: "New note" }), {
    target: { value: "Keep my idea" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save note" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Disk full");
  expect(screen.getByRole("textbox", { name: "New note" })).toHaveValue("Keep my idea");
});

it("persists dismissal of a correction pair without changing vocabulary", async () => {
  const settings = await api.getSettings();
  const save = vi.fn().mockResolvedValue(true);
  render(
    <DictionaryPage
      settings={{
        ...settings,
        dictionary_suggestions: [{ before: "reflow", after: "Reflow", frequency: 2 }],
        dismissed_corrections: [],
      }}
      onUpdateSettings={save}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith({
      dictionary_suggestions: [],
      dismissed_corrections: ["reflow\nReflow"],
    }),
  );
});
