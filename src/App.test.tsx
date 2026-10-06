import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { it, expect, vi } from "vitest";
import { App } from "./App";
import { api } from "./services/tauriApi";
import * as bridge from "./services/tauriApi";

it.each(["app", "model"])(
  "keeps a newer %s event when its startup snapshot arrives late",
  async (source) => {
    const status = await api.getModelStatus();
    const ready = { ...status, installed: true, loaded: true, is_loading: false };
    const listeners = new Map<string, (payload: any) => void>();
    vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
      listeners.set(event, handler);
      return () => {};
    });
    let finishState!: (value: "READY") => void;
    let finishModel!: (value: typeof status) => void;
    vi.spyOn(api, "getAppState").mockImplementation(() =>
      source === "app"
        ? new Promise((resolve) => {
            finishState = resolve;
          })
        : Promise.resolve("READY"),
    );
    vi.spyOn(api, "getModelStatus").mockImplementation(() =>
      source === "model"
        ? new Promise((resolve) => {
            finishModel = resolve;
          })
        : Promise.resolve(ready),
    );
    render(<App />);
    await screen.findByRole("heading", { name: /Speak freely/ });
    await act(async () => {
      if (source === "app") listeners.get("app:state-changed")?.("RECORDING");
      else listeners.get("model:status")?.(ready);
    });
    await act(async () => {
      if (source === "app") finishState("READY");
      else finishModel(status);
    });
    expect(
      screen.getByRole("button", { name: source === "app" ? "Stop recording" : "Start recording" }),
    ).toBeEnabled();
  },
);

it("reconciles an independent settings event after a local save drains", async () => {
  const initial = await api.getSettings();
  const local = { ...initial, language: "hi", auto_detect_language: false };
  const latest = { ...local, accent_color: "rose" as const };
  vi.spyOn(api, "getSettings").mockResolvedValueOnce(initial).mockResolvedValue(latest);
  let finishSave!: (value: typeof initial) => void;
  vi.spyOn(api, "updateSettings").mockImplementation(
    () =>
      new Promise((resolve) => {
        finishSave = resolve;
      }),
  );
  const listeners = new Map<string, (payload: any) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event, handler);
    return () => {};
  });
  render(<App />);
  fireEvent.change(await screen.findByRole("combobox", { name: "Language" }), {
    target: { value: "hi" },
  });
  await waitFor(() => expect(api.updateSettings).toHaveBeenCalled());
  await act(async () => listeners.get("settings:changed")?.(latest));
  await act(async () => finishSave(local));
  await waitFor(() => expect(document.documentElement.dataset.accent).toBe("rose"));
  expect(screen.getByRole("combobox", { name: "Language" })).toHaveValue("hi");
});

it("keeps an import that starts after navigation available for cancellation and reports its failure", async () => {
  let finishStart!: (value: { job_id: string }) => void;
  vi.spyOn(api, "transcribeFile").mockImplementation(
    () =>
      new Promise((resolve) => {
        finishStart = resolve;
      }),
  );
  let finishPoll!: (value: Awaited<ReturnType<typeof api.getFileJob>>) => void;
  vi.spyOn(api, "getFileJob").mockImplementation(
    () =>
      new Promise((resolve) => {
        finishPoll = resolve;
      }),
  );
  const cancel = vi.spyOn(api, "cancelFileTranscription").mockResolvedValue();
  render(<App />);
  fireEvent.change(await screen.findByRole("textbox", { name: "Audio file path" }), {
    target: { value: "C:/long.wav" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Transcribe file" }));
  fireEvent.click(screen.getByRole("button", { name: "History" }));
  await act(async () => finishStart({ job_id: "long-import" }));
  fireEvent.click(screen.getByRole("button", { name: "Home" }));
  fireEvent.click(await screen.findByRole("button", { name: "Cancel import" }));
  await waitFor(() => expect(cancel).toHaveBeenCalledWith("long-import"));
  fireEvent.click(screen.getByRole("button", { name: "Notes" }));
  await act(async () =>
    finishPoll({
      job_id: "long-import",
      done_s: 0,
      total_s: 0,
      finished: true,
      history_id: null,
      error: "Audio decode failed",
    }),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent("Audio decode failed");
  fireEvent.click(screen.getByRole("button", { name: "Home" }));
  expect(screen.getByRole("textbox", { name: "Audio file path" })).toHaveValue("C:/long.wav");
  expect(screen.getByRole("button", { name: "Transcribe file" })).toBeEnabled();
});

it("offers retry when startup settings fail and recovers", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "getSettings")
    .mockRejectedValueOnce(new Error("unavailable"))
    .mockResolvedValue(settings);
  render(<App />);
  await screen.findByRole("button", { name: "Try again" });
  fireEvent.click(screen.getByRole("button", { name: "Try again" }));
  await screen.findByRole("heading", { name: /Speak freely/ });
  expect(screen.queryByText("Starting Reflow…")).not.toBeInTheDocument();
});

it("keeps independent event subscriptions alive after one registration fails and surfaces hotkey errors", async () => {
  const listeners = new Map<string, (payload: any) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    if (event === "model:status") throw new Error("listener unavailable");
    listeners.set(event, handler);
    return () => {};
  });
  render(<App />);
  await screen.findByRole("heading", { name: /Speak freely/ });
  expect(listeners.has("app:state-changed")).toBe(true);
  expect(listeners.has("transcript:final")).toBe(true);
  await act(async () => listeners.get("recording:error")?.("Microphone is unavailable"));
  expect(screen.getByText("Microphone is unavailable")).toBeInTheDocument();
});

it("reports manual reload rejection to the user", async () => {
  const status = await api.getModelStatus();
  vi.spyOn(api, "getModelStatus").mockResolvedValue({
    ...status,
    installed: true,
    loaded: true,
    backend: "CPU",
    is_loading: false,
  });
  vi.spyOn(api, "reloadModel").mockRejectedValue(new Error("sidecar exited"));
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
  fireEvent.click(await screen.findByRole("button", { name: "Performance" }));
  fireEvent.click(await screen.findByRole("button", { name: "Reload speech model" }));
  expect(await screen.findByText(/Could not reload the speech model/)).toBeInTheDocument();
});

it("keeps the shell usable when model status is unavailable", async () => {
  vi.spyOn(api, "getModelStatus").mockRejectedValue(new Error("sidecar unavailable"));
  render(<App />);
  await screen.findByRole("button", { name: "History" });
  fireEvent.click(screen.getByRole("button", { name: "History" }));
  await screen.findByRole("heading", { name: "History" });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 200));
  });
  await waitFor(() => expect(screen.queryByText("Searching transcripts…")).not.toBeInTheDocument());
});

it("serializes preference changes and disables auto-detection for an explicit language", async () => {
  const initial = await api.getSettings();
  let releaseFirst!: (value: typeof initial) => void;
  const update = vi
    .spyOn(api, "updateSettings")
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          releaseFirst = resolve;
        }),
    )
    .mockResolvedValue({ ...initial, language: "hi", auto_detect_language: false });
  render(<App />);
  const language = await screen.findByRole("combobox", { name: "Language" });
  fireEvent.change(language, { target: { value: "en" } });
  await waitFor(() => expect(update).toHaveBeenCalledTimes(1));
  fireEvent.change(language, { target: { value: "hi" } });
  expect(update).toHaveBeenCalledTimes(1);
  await act(async () => {
    releaseFirst({ ...initial, language: "en", auto_detect_language: false });
  });
  await waitFor(() => expect(update).toHaveBeenCalledTimes(2));
  expect(update).toHaveBeenLastCalledWith({ language: "hi", auto_detect_language: false });
  expect(language).toHaveValue("hi");
});

it("preserves a newer note draft when an earlier save finishes after navigating away", async () => {
  const savedNote: Awaited<ReturnType<typeof api.addNote>> = {
    id: "saved-note",
    kind: "note",
    created_at: "2026-10-03T00:00:00Z",
    final_transcript: "The idea being saved",
    raw_transcript: "The idea being saved",
    duration_ms: 0,
    language: "en",
    application_name: "",
    application_process: "",
    word_count: 4,
    character_count: 20,
    model_version: "test",
    processing_mode: "raw",
  };
  let saved = false;
  vi.spyOn(api, "queryHistory").mockImplementation(async () => ({
    entries: saved ? [savedNote] : [],
    total: saved ? 1 : 0,
    next_offset: null,
    recovery_notice: null,
  }));
  let completeSave!: (entry: Awaited<ReturnType<typeof api.addNote>>) => void;
  const saving = vi.spyOn(api, "addNote").mockImplementation(
    () =>
      new Promise((resolve) => {
        completeSave = resolve;
      }),
  );
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "Notes" }));
  fireEvent.change(screen.getByRole("textbox", { name: "New note" }), {
    target: { value: "The idea being saved" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save note" }));
  await waitFor(() => expect(saving).toHaveBeenCalledTimes(1));
  fireEvent.click(screen.getByRole("button", { name: "Home" }));
  fireEvent.click(screen.getByRole("button", { name: "Notes" }));
  const composer = screen.getByRole("textbox", { name: "New note" });
  fireEvent.change(composer, { target: { value: "A newer idea I am still writing" } });
  await screen.findByText("Make room for your ideas");
  await act(async () => {
    saved = true;
    completeSave(savedNote);
  });

  expect(await screen.findByText("The idea being saved", { selector: "p" })).toBeInTheDocument();
  expect(composer).toHaveValue("A newer idea I am still writing");
  expect(screen.getByRole("button", { name: "Save note" })).toBeEnabled();
});

it("opens settings as a dialog over the current page and closes it", async () => {
  render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "Settings" }));
  expect(await screen.findByRole("dialog", { name: "Settings" })).toBeInTheDocument();
  expect(screen.getByText("Speak freely.")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Close settings" }));
  expect(screen.queryByRole("dialog", { name: "Settings" })).not.toBeInTheDocument();
});

it("opens the settings dialog when the tray navigates to settings", async () => {
  const listeners = new Map<string, (payload: any) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event, handler);
    return () => {};
  });
  render(<App />);
  await screen.findByRole("heading", { name: /Speak freely/ });
  await act(async () => listeners.get("ui:navigate")?.("settings"));
  expect(screen.getByRole("dialog", { name: "Settings" })).toBeInTheDocument();
});

it("collapses the sidebar and remembers the choice", async () => {
  localStorage.removeItem("reflow.sidebar-collapsed");
  const { container } = render(<App />);
  fireEvent.click(await screen.findByRole("button", { name: "Collapse sidebar" }));
  expect(container.querySelector("#app-sidebar")).toHaveAttribute("data-collapsed", "true");
  expect(localStorage.getItem("reflow.sidebar-collapsed")).toBe("1");
  expect(screen.getByRole("button", { name: "Expand sidebar" })).toHaveAttribute(
    "aria-expanded",
    "false",
  );
  localStorage.removeItem("reflow.sidebar-collapsed");
});
