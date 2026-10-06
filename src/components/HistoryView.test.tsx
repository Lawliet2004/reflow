import { act, render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { HistoryView } from "./HistoryView";
import { api } from "../services/tauriApi";
import * as bridge from "../services/tauriApi";
import type { HistoryEntry } from "../types";

function entry(): HistoryEntry {
  const date = new Date();
  date.setDate(date.getDate() - 2);
  date.setHours(12, 0, 0, 0);
  return {
    id: "one",
    created_at: date.toISOString(),
    final_transcript: "Keep this thought",
    raw_transcript: "Keep this thought",
    smart_transcript: "",
    rewriter_used: false,
    duration_ms: 1000,
    language: "en",
    application_name: "",
    application_process: "",
    word_count: 3,
    character_count: 17,
    model_version: "test",
    processing_mode: "raw",
  };
}

it("refreshes corrected history from another app without losing an open draft", async () => {
  const listeners = new Map<string, (payload: HistoryEntry) => void>();
  const unsubscribe = vi.fn();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event, handler);
    return unsubscribe;
  });
  const query = vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [entry()],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  const { unmount } = render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "Edit transcript" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Edit transcript" }), {
    target: { value: "My unsaved draft" },
  });
  await act(async () => listeners.get("history:updated")?.(entry()));
  await waitFor(() => expect(query).toHaveBeenCalledTimes(2));
  expect(screen.getByRole("textbox", { name: "Edit transcript" })).toHaveValue("My unsaved draft");
  unmount();
  expect(unsubscribe).toHaveBeenCalled();
});

it("refreshes the current search when an earlier deletion finishes", async () => {
  const second = { ...entry(), id: "two", final_transcript: "Another thought" };
  let finishSearch!: (value: Awaited<ReturnType<typeof api.queryHistory>>) => void;
  const searchResult = new Promise<Awaited<ReturnType<typeof api.queryHistory>>>((resolve) => {
    finishSearch = resolve;
  });
  const query = vi
    .spyOn(api, "queryHistory")
    .mockImplementation(async (filter) =>
      filter.query
        ? searchResult
        : { entries: [entry()], total: 1, next_offset: null, recovery_notice: null },
    );
  let finishDelete!: (value: boolean) => void;
  vi.spyOn(api, "deleteHistoryItem").mockImplementation(
    () =>
      new Promise((resolve) => {
        finishDelete = resolve;
      }),
  );
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "Delete transcript" }));
  fireEvent.change(screen.getByRole("textbox", { name: "Search transcripts" }), {
    target: { value: "Another" },
  });
  await waitFor(() => expect(query).toHaveBeenCalledTimes(2));
  await act(async () => finishDelete(true));
  await act(async () =>
    finishSearch({ entries: [second], total: 1, next_offset: null, recovery_notice: null }),
  );
  expect(await screen.findByText("Another thought")).toBeInTheDocument();
  expect(screen.getByText("1 entry")).toBeInTheDocument();
});

it.each(["transcript", "tags"])(
  "preserves a newer %s draft typed while the same editor is saving",
  async (kind) => {
    vi.spyOn(api, "queryHistory").mockResolvedValue({
      entries: [entry()],
      total: 1,
      next_offset: null,
      recovery_notice: null,
    });
    let finishEdit!: (value: HistoryEntry) => void;
    let finishTags!: () => void;
    const edit = vi.spyOn(api, "editHistoryTranscript").mockImplementation(
      () =>
        new Promise((resolve) => {
          finishEdit = resolve;
        }),
    );
    const tags = vi.spyOn(api, "updateHistoryMetadata").mockImplementation(
      () =>
        new Promise((resolve) => {
          finishTags = resolve;
        }),
    );
    render(<HistoryView />);
    await screen.findByText("Keep this thought");
    fireEvent.click(screen.getByRole("button", { name: "More options" }));
    fireEvent.click(
      screen.getByRole("menuitem", { name: kind === "tags" ? "Edit tags" : "Edit transcript" }),
    );
    const editor = screen.getByRole("textbox", {
      name: kind === "tags" ? "Transcript tags" : "Edit transcript",
    });
    fireEvent.change(editor, { target: { value: "Submitted draft A" } });
    fireEvent.click(
      screen.getByRole("button", { name: kind === "tags" ? "Save tags" : "Save edit" }),
    );
    if (kind === "tags") expect(tags).toHaveBeenCalledWith("one", false, "Submitted draft A");
    else expect(edit).toHaveBeenCalledWith("one", "Submitted draft A");
    fireEvent.change(editor, { target: { value: "Unsaved draft B" } });
    await act(async () => {
      if (kind === "tags") finishTags();
      else finishEdit({ ...entry(), final_transcript: "Submitted draft A" });
    });
    expect(editor).toBeInTheDocument();
    expect(editor).toHaveValue("Unsaved draft B");
    expect(
      screen.getByRole("button", { name: kind === "tags" ? "Save tags" : "Save edit" }),
    ).toBeEnabled();
  },
);

it.each(["edit", "retry"])(
  "refreshes the current filter when %s completes before an older query",
  async (kind) => {
    const original = { ...entry(), audio_available: true, audio_expires_at: null };
    const updated = { ...original, final_transcript: "Updated thought" };
    let finishQuery!: (value: Awaited<ReturnType<typeof api.queryHistory>>) => void;
    const pendingQuery = new Promise<Awaited<ReturnType<typeof api.queryHistory>>>((resolve) => {
      finishQuery = resolve;
    });
    const result = (value: HistoryEntry) => ({
      entries: [value],
      total: 1,
      next_offset: null,
      recovery_notice: null,
    });
    const query = vi
      .spyOn(api, "queryHistory")
      .mockResolvedValueOnce(result(original))
      .mockReturnValueOnce(pendingQuery)
      .mockResolvedValue(result(updated));
    let finishMutation!: (value: HistoryEntry) => void;
    const pendingMutation = new Promise<HistoryEntry>((resolve) => {
      finishMutation = resolve;
    });
    vi.spyOn(api, "editHistoryTranscript").mockReturnValue(pendingMutation);
    vi.spyOn(api, "retryHistoryTranscript").mockReturnValue(pendingMutation);
    render(<HistoryView />);
    await screen.findByText("Keep this thought");
    fireEvent.click(screen.getByRole("button", { name: "More options" }));
    fireEvent.click(
      screen.getByRole("menuitem", {
        name: kind === "edit" ? "Edit transcript" : "Retry transcript",
      }),
    );
    if (kind === "edit") {
      fireEvent.change(screen.getByRole("textbox", { name: "Edit transcript" }), {
        target: { value: updated.final_transcript },
      });
      fireEvent.click(screen.getByRole("button", { name: "Save edit" }));
    }
    fireEvent.change(screen.getByRole("textbox", { name: "Search transcripts" }), {
      target: { value: "thought" },
    });
    await waitFor(() => expect(query).toHaveBeenCalledTimes(2));
    await act(async () => {
      finishMutation(updated);
      await Promise.resolve();
      finishQuery(result(original));
    });
    expect(screen.getByText("Updated thought")).toBeInTheDocument();
    await waitFor(() => expect(query).toHaveBeenCalledTimes(3));
    expect(query).toHaveBeenLastCalledWith(expect.objectContaining({ query: "thought" }));
    expect(screen.getByText("Updated thought")).toBeInTheDocument();
  },
);

it.each(["transcript", "tags"])(
  "keeps a newer %s editor open when an earlier save completes",
  async (kind) => {
    const second = { ...entry(), id: "two", final_transcript: "Another thought" };
    vi.spyOn(api, "queryHistory").mockResolvedValue({
      entries: [entry(), second],
      total: 2,
      next_offset: null,
      recovery_notice: null,
    });
    let finishEdit!: (value: HistoryEntry) => void;
    let finishTags!: () => void;
    vi.spyOn(api, "editHistoryTranscript").mockImplementation(
      () =>
        new Promise((resolve) => {
          finishEdit = resolve;
        }),
    );
    vi.spyOn(api, "updateHistoryMetadata").mockImplementation(
      () =>
        new Promise((resolve) => {
          finishTags = resolve;
        }),
    );
    render(<HistoryView />);
    await screen.findByText("Keep this thought");
    const openEditor = (text: string) => {
      fireEvent.click(
        within(screen.getByText(text).closest(".group")!).getByRole("button", {
          name: "More options",
        }),
      );
      fireEvent.click(
        screen.getByRole("menuitem", { name: kind === "tags" ? "Edit tags" : "Edit transcript" }),
      );
    };
    openEditor("Keep this thought");
    fireEvent.click(
      screen.getByRole("button", { name: kind === "tags" ? "Save tags" : "Save edit" }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: kind === "tags" ? "Cancel tags" : "Cancel" }),
    );
    openEditor("Another thought");
    const editor = screen.getByRole("textbox", {
      name: kind === "tags" ? "Transcript tags" : "Edit transcript",
    });
    fireEvent.change(editor, { target: { value: "A newer draft" } });
    await act(async () => {
      if (kind === "tags") finishTags();
      else finishEdit(entry());
    });
    expect(editor).toBeInTheDocument();
    expect(editor).toHaveValue("A newer draft");
  },
);
it("includes the day before yesterday in Earlier", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [entry()],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "Earlier" }));
  expect(screen.getByText("Keep this thought")).toBeInTheDocument();
});
it("keeps a transcript visible when deletion fails", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [entry()],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  vi.spyOn(api, "deleteHistoryItem").mockRejectedValue(new Error("database locked"));
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "Delete transcript" }));
  await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("Could not delete"));
  expect(screen.getByText("Keep this thought")).toBeInTheDocument();
});

it("keeps original and cleaned views without an Undo AI edit action", async () => {
  const edited = {
    ...entry(),
    raw_transcript: "uh keep this thought",
    smart_transcript: "Keep this thought",
    final_transcript: "Keep this thought, elegantly.",
    rewriter_used: true,
  };
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [edited],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  render(<HistoryView />);
  await screen.findByText("Keep this thought, elegantly.");
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  expect(screen.queryByRole("menuitem", { name: "Undo AI edit" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Show original" }));
  expect(screen.getByText("uh keep this thought")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Show cleaned" }));
  expect(screen.getByText("Keep this thought, elegantly.")).toBeInTheDocument();
});

it("hides audio actions for legacy transcripts with no saved audio", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [entry()],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  expect(screen.queryByRole("menuitem", { name: "Undo AI edit" })).not.toBeInTheDocument();
  expect(screen.queryByRole("menuitem", { name: "Retry transcript" })).not.toBeInTheDocument();
  expect(screen.queryByRole("menuitem", { name: "Extract audio" })).not.toBeInTheDocument();
  expect(screen.getByRole("menuitem", { name: "Delete transcript" })).toBeEnabled();
});

it.each([false, true])("offers audio actions only before expiry (expired: %s)", async (expired) => {
  const saved = {
    ...entry(),
    audio_available: true,
    audio_expires_at: Date.now() + (expired ? -1000 : 60_000),
  };
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [saved],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  for (const name of ["Retry transcript", "Extract audio"]) {
    if (expired) expect(screen.queryByRole("menuitem", { name })).not.toBeInTheDocument();
    else expect(screen.getByRole("menuitem", { name })).toBeEnabled();
  }
});

it("removes audio actions when a recording expires while its menu is open", async () => {
  vi.useFakeTimers();
  const saved = { ...entry(), audio_available: true, audio_expires_at: Date.now() + 1000 };
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [saved],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  render(<HistoryView />);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(200);
  });
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  expect(screen.getByRole("menuitem", { name: "Extract audio" })).toBeEnabled();
  screen.getByRole("menuitem", { name: "Extract audio" }).focus();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(1000);
  });
  expect(screen.queryByRole("menuitem", { name: "Extract audio" })).not.toBeInTheDocument();
  expect(screen.queryByRole("menuitem", { name: "Retry transcript" })).not.toBeInTheDocument();
  expect(screen.getByRole("menuitem", { name: "Delete transcript" })).toBeEnabled();
  expect(screen.getByRole("menuitem", { name: "Edit transcript" })).toHaveFocus();
});

it("extracts audio and reports its saved path", async () => {
  const saved = { ...entry(), audio_available: true, audio_expires_at: null };
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [saved],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  const extract = vi
    .spyOn(api, "extractHistoryAudio")
    .mockResolvedValue("Downloads/Reflow-audio.wav");
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "Extract audio" }));
  expect(await screen.findByRole("status")).toHaveTextContent(
    "Audio exported to Downloads/Reflow-audio.wav",
  );
  expect(extract).toHaveBeenCalledWith("one");
});

it("preserves the transcript when retry fails", async () => {
  const saved = { ...entry(), audio_available: true, audio_expires_at: null };
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [saved],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  vi.spyOn(api, "retryHistoryTranscript").mockRejectedValue(new Error("Speech model is not ready"));
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  fireEvent.click(screen.getByRole("menuitem", { name: "Retry transcript" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Speech model is not ready");
  expect(screen.getByText("Keep this thought")).toBeInTheDocument();
});

it("offers a single retry action without retry options", async () => {
  const saved = { ...entry(), audio_available: true, audio_expires_at: null };
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [saved],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  const retry = vi.spyOn(api, "retryHistoryTranscript").mockResolvedValue(saved);
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  expect(screen.queryByRole("menuitem", { name: "Retry with options" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("menuitem", { name: "Retry transcript" }));
  await waitFor(() => expect(retry).toHaveBeenCalledWith("one"));
});

it("renders transcript actions outside the clipped history panel", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [entry()],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  const { container } = render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  const menu = screen.getByRole("menu", { name: "Transcript actions" });
  expect(container).not.toContainElement(menu);
  expect(document.body).toContainElement(menu);
});

it("moves through menu actions with the keyboard and restores focus on Escape", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [entry()],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  const trigger = screen.getByRole("button", { name: "More options" });
  trigger.focus();
  fireEvent.keyDown(trigger, { key: "ArrowDown" });
  expect(screen.getByRole("menuitem", { name: "Edit transcript" })).toHaveFocus();
  fireEvent.keyDown(document.activeElement!, { key: "End" });
  expect(screen.getByRole("menuitem", { name: "Delete transcript" })).toHaveFocus();
  fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
  expect(screen.getByRole("menuitem", { name: "Edit transcript" })).toHaveFocus();
  fireEvent.keyDown(document.activeElement!, { key: "Escape" });
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  expect(trigger).toHaveFocus();
});

it("dismisses transcript actions when clicking outside", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [entry()],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  fireEvent.pointerDown(screen.getByRole("textbox", { name: "Search transcripts" }));
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
});

it("continues keyboard tab order from the menu trigger", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [entry()],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  fireEvent.keyDown(document.activeElement!, { key: "Tab", shiftKey: true });
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Copy transcript" })).toHaveFocus();
});

it("wraps Tab from the final transcript menu to the first page control", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [entry()],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  fireEvent.keyDown(document.activeElement!, { key: "Tab" });
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Export matching" })).toHaveFocus();
});

it("skips hidden ancestors, closed details, and controls outside the tab order", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [entry()],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  render(
    <>
      <HistoryView />
      <div hidden>
        <button>Hidden ancestor control</button>
      </div>
      <div aria-hidden="true">
        <button>Hidden from accessibility</button>
      </div>
      <div style={{ display: "none" }}>
        <button>Hidden by styles</button>
      </div>
      <details>
        <summary tabIndex={-1}>Closed section</summary>
        <button>Closed details control</button>
      </details>
      <button tabIndex={-1}>Outside tab order</button>
      <button>Next visible control</button>
    </>,
  );
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "More options" }));
  fireEvent.keyDown(document.activeElement!, { key: "Tab" });
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Next visible control" })).toHaveFocus();
});

it("keeps a tall menu within the viewport and repositions it after a resize", async () => {
  vi.spyOn(api, "queryHistory").mockResolvedValue({
    entries: [entry()],
    total: 1,
    next_offset: null,
    recovery_notice: null,
  });
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  const trigger = screen.getByRole("button", { name: "More options" });
  vi.spyOn(trigger, "getBoundingClientRect").mockReturnValue({
    x: 280,
    y: 290,
    left: 280,
    right: 312,
    top: 290,
    bottom: 322,
    width: 32,
    height: 32,
    toJSON: () => ({}),
  });
  vi.stubGlobal("innerWidth", 320);
  vi.stubGlobal("innerHeight", 340);
  try {
    fireEvent.click(trigger);
    const menu = screen.getByRole("menu", { name: "Transcript actions" });
    Object.defineProperty(menu, "scrollHeight", { configurable: true, value: 380 });
    fireEvent(window, new Event("resize"));
    expect(menu).toHaveStyle({ top: "8px", left: "88px", maxHeight: "276px" });
    vi.stubGlobal("innerWidth", 200);
    vi.stubGlobal("innerHeight", 180);
    fireEvent(window, new Event("resize"));
    expect(menu).toHaveStyle({ top: "8px", left: "8px", maxHeight: "164px" });
  } finally {
    vi.unstubAllGlobals();
  }
});

it("queries dates and search before paging and displays the database total", async () => {
  const query = vi
    .spyOn(api, "queryHistory")
    .mockResolvedValue({ entries: [entry()], total: 120, next_offset: 50, recovery_notice: null });
  render(<HistoryView />);
  await screen.findByText("120 entries");
  fireEvent.click(screen.getByRole("button", { name: "Yesterday" }));
  await waitFor(() =>
    expect(query).toHaveBeenLastCalledWith(
      expect.objectContaining({ from: expect.any(String), until: expect.any(String), offset: 0 }),
    ),
  );
  fireEvent.change(screen.getByRole("textbox", { name: "Search transcripts" }), {
    target: { value: "%_" },
  });
  await waitFor(() =>
    expect(query).toHaveBeenLastCalledWith(expect.objectContaining({ query: "%_", offset: 0 })),
  );
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
  await waitFor(() =>
    expect(query).toHaveBeenLastCalledWith(expect.objectContaining({ query: "%_", offset: 50 })),
  );
  fireEvent.click(screen.getByRole("button", { name: "Clear search" }));
  await waitFor(() =>
    expect(query).toHaveBeenLastCalledWith(expect.objectContaining({ query: "", offset: 0 })),
  );
});
