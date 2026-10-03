import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import userEvent from "@testing-library/user-event";
import { api } from "../services/tauriApi";
import { NotesView } from "./NotesView";
import type { HistoryEntry, HistoryPage } from "../types";

function note(text = "Keep this idea"): HistoryEntry {
  return {
    id: "note-one",
    kind: "note",
    created_at: "2026-10-02T12:00:00Z",
    final_transcript: text,
    raw_transcript: text,
    duration_ms: 0,
    language: "en",
    application_name: "",
    application_process: "",
    word_count: 3,
    character_count: text.length,
    model_version: "test",
    processing_mode: "raw",
  };
}

function page(entries: HistoryEntry[] = []): HistoryPage {
  return { entries, total: entries.length, next_offset: null, recovery_notice: null };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}

it("waits for notes to load before showing an empty collection and hides unnecessary pagination", async () => {
  const result = deferred<HistoryPage>();
  const query = vi.spyOn(api, "queryHistory").mockReturnValue(result.promise);
  render(<NotesView settings={null} onUpdateSettings={vi.fn()} />);

  expect(
    screen.queryByText(/your saved notes will appear|no notes yet|your first note/i),
  ).toBeNull();
  expect(screen.getByText(/loading notes/i)).toBeInTheDocument();
  await waitFor(() => expect(query).toHaveBeenCalled());
  await act(async () => result.resolve(page()));

  expect(screen.queryByText(/loading notes/i)).toBeNull();
  expect(screen.queryByRole("button", { name: "Previous page" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Next page" })).toBeNull();
});

it("reports a failed load and lets the user recover without leaving Notes", async () => {
  vi.spyOn(api, "queryHistory")
    .mockRejectedValueOnce(new Error("Notes database is unavailable"))
    .mockResolvedValue(page([note()]));
  render(<NotesView settings={null} onUpdateSettings={vi.fn()} />);

  expect(await screen.findByRole("alert")).toHaveTextContent("Notes database is unavailable");
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));

  expect(await screen.findByText("Keep this idea")).toBeInTheDocument();
  expect(screen.queryByRole("alert")).toBeNull();
});

it("preserves a failed draft and prevents duplicate saves while the request is pending", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue(page());
  const saving = deferred<HistoryEntry>();
  const addNote = vi.spyOn(api, "addNote").mockReturnValue(saving.promise);
  render(<NotesView settings={null} onUpdateSettings={vi.fn()} />);
  fireEvent.change(screen.getByRole("textbox", { name: "New note" }), {
    target: { value: "A thought I must keep" },
  });
  const save = screen.getByRole("button", { name: "Save note" });
  fireEvent.click(save);
  fireEvent.click(save);
  expect(save).toBeDisabled();
  expect(addNote).toHaveBeenCalledTimes(1);
  await act(async () => saving.reject(new Error("Disk full")));

  expect(screen.getByRole("alert")).toHaveTextContent("Disk full");
  expect(screen.getByRole("textbox", { name: "New note" })).toHaveValue("A thought I must keep");
  expect(screen.getByRole("button", { name: "Save note" })).toBeEnabled();
});

it("resynchronizes the export folder after settings change and explicitly reports a failed folder save", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue(page());
  const settings = await api.getSettings();
  const update = vi.fn().mockResolvedValue(false);
  const { rerender } = render(
    <NotesView settings={{ ...settings, notes_folder: "C:/notes" }} onUpdateSettings={update} />,
  );
  fireEvent.click(screen.getByText("Export location"));
  const folder = screen.getByRole("textbox", { name: "Export folder" });
  expect(folder).toHaveValue("C:/notes");
  rerender(
    <NotesView settings={{ ...settings, notes_folder: "D:/ideas" }} onUpdateSettings={update} />,
  );
  expect(folder).toHaveValue("D:/ideas");
  fireEvent.change(folder, { target: { value: "D:/journal" } });
  fireEvent.blur(folder);
  expect(update).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Save folder" }));

  await waitFor(() => expect(update).toHaveBeenCalledWith({ notes_folder: "D:/journal" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(/folder/i);
  expect(folder).toHaveValue("D:/journal");
});

it("edits a saved note inline and keeps the draft available if the edit fails", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue(page([note()]));
  const edit = vi.spyOn(api, "editHistoryTranscript").mockRejectedValue(new Error("Disk full"));
  render(<NotesView settings={null} onUpdateSettings={vi.fn()} />);
  await screen.findByText("Keep this idea");
  fireEvent.click(screen.getByRole("button", { name: "Edit note" }));
  const editor = screen.getByRole("textbox", { name: "Edit note text" });
  expect(editor).toHaveValue("Keep this idea");
  fireEvent.change(editor, { target: { value: "My revised idea" } });
  fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("Disk full");
  expect(edit).toHaveBeenCalledWith("note-one", "My revised idea");
  expect(editor).toHaveValue("My revised idea");
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.getByText("Keep this idea")).toBeInTheDocument();
  expect(screen.queryByRole("textbox", { name: "Edit note text" })).toBeNull();
});

it("requires an inline delete confirmation and preserves the note when deletion fails", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue(page([note()]));
  const remove = vi.spyOn(api, "deleteHistoryItem").mockRejectedValue(new Error("Database locked"));
  render(<NotesView settings={null} onUpdateSettings={vi.fn()} />);
  const article = (await screen.findByText("Keep this idea")).closest("article")!;
  fireEvent.click(within(article).getByRole("button", { name: "Delete note" }));
  expect(remove).not.toHaveBeenCalled();
  expect(within(article).getByText("Delete note?")).toBeInTheDocument();
  fireEvent.click(within(article).getByRole("button", { name: "Keep note" }));
  expect(within(article).queryByText("Delete note?")).toBeNull();
  fireEvent.click(within(article).getByRole("button", { name: "Delete note" }));
  fireEvent.click(within(article).getByRole("button", { name: "Delete note" }));

  expect(await screen.findByRole("alert")).toHaveTextContent("Database locked");
  expect(screen.getByText("Keep this idea")).toBeInTheDocument();
});

it("reflects recording and processing state supplied by the app", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue(page());
  const start = vi.spyOn(api, "startRecording").mockResolvedValue();
  const stop = vi.spyOn(api, "stopRecording").mockResolvedValue("");
  const update = vi.fn();
  const { rerender } = render(
    <NotesView settings={null} onUpdateSettings={update} appState="READY" />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Record note" }));
  await waitFor(() => expect(start).toHaveBeenCalledWith("note"));
  rerender(<NotesView settings={null} onUpdateSettings={update} appState="RECORDING" />);
  fireEvent.click(screen.getByRole("button", { name: "Stop recording" }));
  await waitFor(() => expect(stop).toHaveBeenCalledTimes(1));
  rerender(<NotesView settings={null} onUpdateSettings={update} appState="PROCESSING" />);

  expect(screen.getByRole("button", { name: "Processing note…" })).toBeDisabled();
});

it("searches within notes and resets paging when the search changes", async () => {
  const query = vi.spyOn(api, "queryHistory").mockResolvedValue({
    ...page([note()]),
    total: 51,
    next_offset: 50,
  });
  render(<NotesView settings={null} onUpdateSettings={vi.fn()} />);
  await screen.findByText("Keep this idea");
  expect(query).toHaveBeenLastCalledWith(
    expect.objectContaining({ kind: "note", limit: 50, offset: 0 }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Next page" }));
  await waitFor(() =>
    expect(query).toHaveBeenLastCalledWith(expect.objectContaining({ offset: 50 })),
  );
  fireEvent.change(screen.getByRole("searchbox", { name: "Search notes" }), {
    target: { value: "journal" },
  });

  await waitFor(() =>
    expect(query).toHaveBeenLastCalledWith(
      expect.objectContaining({ kind: "note", query: "journal", limit: 50, offset: 0 }),
    ),
  );
});

it("saves a new note into the collection and clears its draft", async () => {
  let saved: HistoryEntry[] = [];
  vi.spyOn(api, "queryHistory").mockImplementation(async () => page(saved));
  vi.spyOn(api, "addNote").mockImplementation(async (text) => {
    const entry = note(text);
    saved = [entry];
    return entry;
  });
  render(<NotesView settings={null} onUpdateSettings={vi.fn()} />);
  await screen.findByText("Make room for your ideas");
  const composer = screen.getByRole("textbox", { name: "New note" });
  fireEvent.change(composer, { target: { value: "A thought worth keeping" } });
  fireEvent.click(screen.getByRole("button", { name: "Save note" }));

  expect(await screen.findByText("A thought worth keeping", { selector: "p" })).toBeInTheDocument();
  expect(composer).toHaveValue("");
  expect(composer).toHaveFocus();
  expect(screen.getByRole("status")).toHaveTextContent("Note saved");
});

it("shows the updated saved text after editing a note", async () => {
  let saved = note();
  vi.spyOn(api, "queryHistory").mockImplementation(async () => page([saved]));
  vi.spyOn(api, "editHistoryTranscript").mockImplementation(async (_id, text) => {
    saved = { ...saved, final_transcript: text };
    return saved;
  });
  render(<NotesView settings={null} onUpdateSettings={vi.fn()} />);
  await screen.findByText("Keep this idea");
  fireEvent.click(screen.getByRole("button", { name: "Edit note" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Edit note text" }), {
    target: { value: "An improved idea" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

  expect(await screen.findByText("An improved idea", { selector: "p" })).toBeInTheDocument();
  expect(screen.queryByText("Keep this idea")).toBeNull();
  expect(screen.queryByRole("textbox", { name: "Edit note text" })).toBeNull();
});

it("reflects a pinned note and confirms copying its saved text", async () => {
  userEvent.setup();
  const copy = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  let saved = note();
  vi.spyOn(api, "queryHistory").mockImplementation(async () => page([saved]));
  vi.spyOn(api, "updateHistoryMetadata").mockImplementation(async (_id, pinned, tags) => {
    saved = { ...saved, pinned, tags };
  });
  render(<NotesView settings={null} onUpdateSettings={vi.fn()} />);
  await screen.findByText("Keep this idea");
  fireEvent.click(screen.getByRole("button", { name: "Pin note" }));

  expect(await screen.findByRole("button", { name: "Unpin note" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(screen.getByText("Pinned")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Copy note" }));
  expect(await screen.findByRole("button", { name: "Copied" })).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("Copied");
  expect(copy).toHaveBeenCalledWith("Keep this idea");
});

it("returns to the previous page when the last note on a page is deleted", async () => {
  let deleted = false;
  const first = { ...note("An earlier idea"), id: "note-earlier" };
  vi.spyOn(api, "queryHistory").mockImplementation(async (query) => {
    if (query.offset === 50) {
      return { ...page(deleted ? [] : [note()]), total: deleted ? 50 : 51 };
    }
    return { ...page([first]), total: deleted ? 50 : 51, next_offset: deleted ? null : 50 };
  });
  vi.spyOn(api, "deleteHistoryItem").mockImplementation(async () => {
    deleted = true;
    return true;
  });
  render(<NotesView settings={null} onUpdateSettings={vi.fn()} />);
  await screen.findByText("An earlier idea");
  fireEvent.click(screen.getByRole("button", { name: "Next page" }));
  await screen.findByText("Keep this idea");
  fireEvent.click(screen.getByRole("button", { name: "Delete note" }));
  fireEvent.click(screen.getByRole("button", { name: "Delete note" }));

  expect(await screen.findByText("An earlier idea")).toBeInTheDocument();
  expect(screen.queryByText("Keep this idea")).toBeNull();
  expect(screen.queryByRole("button", { name: "Previous page" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Next page" })).toBeNull();
});

it("exports all saved notes even when the current search has no matches", async () => {
  vi.spyOn(api, "queryHistory").mockImplementation(async (query) =>
    page(query.query ? [] : [note()]),
  );
  vi.spyOn(api, "exportNotes").mockResolvedValue("D:/notes/export.md");
  render(<NotesView settings={null} onUpdateSettings={vi.fn()} />);
  await screen.findByText("Keep this idea");
  fireEvent.change(screen.getByRole("searchbox", { name: "Search notes" }), {
    target: { value: "missing thought" },
  });
  await screen.findByText("No notes found");
  const exportButton = screen.getByRole("button", { name: "Export Markdown" });
  expect(exportButton).toBeEnabled();
  fireEvent.click(exportButton);

  expect(await screen.findByRole("status")).toHaveTextContent("D:/notes/export.md");
});

it("lets the user stop recording while a Markdown export is still pending", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue(page([note()]));
  const exporting = deferred<string>();
  vi.spyOn(api, "exportNotes").mockReturnValue(exporting.promise);
  const stop = vi.spyOn(api, "stopRecording").mockResolvedValue("");
  render(<NotesView settings={null} onUpdateSettings={vi.fn()} appState="RECORDING" />);
  await screen.findByText("Keep this idea");
  fireEvent.click(screen.getByRole("button", { name: "Export Markdown" }));
  expect(screen.getByRole("button", { name: "Exporting…" })).toBeDisabled();
  const stopButton = screen.getByRole("button", { name: "Stop recording" });
  expect(stopButton).toBeEnabled();
  fireEvent.click(stopButton);
  await waitFor(() => expect(stop).toHaveBeenCalledTimes(1));
  await act(async () => exporting.resolve("D:/notes/export.md"));

  expect(screen.getByRole("status")).toHaveTextContent("D:/notes/export.md");
});

it("focuses the safe delete confirmation action and returns focus after canceling", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue(page([note()]));
  render(<NotesView settings={null} onUpdateSettings={vi.fn()} />);
  await screen.findByText("Keep this idea");
  const deleteButton = screen.getByRole("button", { name: "Delete note" });
  deleteButton.focus();
  fireEvent.click(deleteButton);
  const keepButton = screen.getByRole("button", { name: "Keep note" });
  await waitFor(() => expect(keepButton).toHaveFocus());
  fireEvent.click(keepButton);

  await waitFor(() => expect(deleteButton).toHaveFocus());
  expect(screen.queryByText("Delete note?")).toBeNull();
});
