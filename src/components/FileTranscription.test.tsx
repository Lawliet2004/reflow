import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { api } from "../services/tauriApi";
import { FileTranscription } from "./FileTranscription";
import * as bridge from "../services/tauriApi";

it.each(["resolve", "reject"])(
  "keeps completed progress events when an older job poll arrives late (%s)",
  async (late) => {
    const listeners = new Map<string, (value: any) => void>();
    vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
      listeners.set(event, handler);
      return () => {};
    });
    const pending = {
      job_id: "event-job",
      done_s: 0,
      total_s: 10,
      finished: false,
      error: null,
      history_id: null,
    };
    let finishRead!: (value: typeof pending) => void;
    let failRead!: (error: Error) => void;
    vi.spyOn(api, "transcribeFile").mockResolvedValue({ job_id: "event-job" });
    const read = vi.spyOn(api, "getFileJob").mockImplementation(
      () =>
        new Promise((resolve, reject) => {
          finishRead = resolve;
          failRead = reject;
        }),
    );
    render(<FileTranscription onOpenHistory={vi.fn()} />);
    fireEvent.change(screen.getByRole("textbox", { name: "Audio file path" }), {
      target: { value: "/tmp/speech.wav" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Transcribe file" }));
    await waitFor(() => expect(read).toHaveBeenCalledOnce());
    await act(async () => {
      listeners.get("transcribe:progress")?.({
        ...pending,
        done_s: 10,
        finished: true,
        history_id: "history-job",
      });
      if (late === "resolve") finishRead(pending);
      else failRead(new Error("Outdated job poll failed"));
    });
    expect(screen.queryByText("Outdated job poll failed")).not.toBeInTheDocument();
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Transcribe file" })).toBeEnabled();
  },
);

it("keeps an empty path and import options unavailable during dictation", () => {
  const { rerender } = render(<FileTranscription onOpenHistory={vi.fn()} />);
  fireEvent.change(screen.getByRole("textbox", { name: "Audio file path" }), {
    target: { value: "   " },
  });
  expect(screen.getByRole("button", { name: "Transcribe file" })).toBeDisabled();

  rerender(<FileTranscription onOpenHistory={vi.fn()} disabled />);
  expect(screen.getByRole("textbox", { name: "Audio file path" })).toBeDisabled();
  expect(screen.getByRole("button", { name: /Browse/ })).toBeDisabled();
  expect(screen.getByRole("checkbox")).toBeDisabled();
  expect(screen.getByText("Finish your current recording before importing a file.")).toBeVisible();
});

it("shows preparation as indeterminate and locks options for the running job", async () => {
  vi.spyOn(api, "transcribeFile").mockResolvedValue({ job_id: "preparing" });
  vi.spyOn(api, "getFileJob").mockResolvedValue({
    job_id: "preparing",
    done_s: 0,
    total_s: 0,
    finished: false,
    error: null,
    history_id: null,
  });
  render(<FileTranscription onOpenHistory={vi.fn()} />);
  fireEvent.change(screen.getByRole("textbox", { name: "Audio file path" }), {
    target: { value: "C:/audio.wav" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Transcribe file" }));

  expect(await screen.findByText("Preparing audio…")).toBeVisible();
  expect(
    screen.getByRole("progressbar", { name: "File transcription progress" }),
  ).not.toHaveAttribute("value");
  expect(screen.queryByText("0 / 0 seconds")).not.toBeInTheDocument();
  expect(screen.getByRole("checkbox")).toBeDisabled();
});

it("trims a pasted path and clears completion when another file is selected", async () => {
  const start = vi.spyOn(api, "transcribeFile").mockResolvedValue({ job_id: "complete" });
  vi.spyOn(api, "getFileJob").mockResolvedValue({
    job_id: "complete",
    done_s: 30,
    total_s: 30,
    finished: true,
    error: null,
    history_id: "transcript",
  });
  render(<FileTranscription onOpenHistory={vi.fn()} />);
  fireEvent.change(screen.getByRole("textbox", { name: "Audio file path" }), {
    target: { value: "  C:/audio.wav  " },
  });
  fireEvent.click(screen.getByRole("button", { name: "Transcribe file" }));
  await screen.findByRole("button", { name: "Open completed transcript in History" });
  expect(start).toHaveBeenCalledWith("C:/audio.wav", false);

  fireEvent.change(screen.getByRole("textbox", { name: "Audio file path" }), {
    target: { value: "C:/next.wav" },
  });
  expect(screen.queryByRole("button", { name: /completed transcript/ })).not.toBeInTheDocument();
});

it("keeps cancellation pending until the job stops and reports cancellation as a normal status", async () => {
  vi.spyOn(api, "transcribeFile").mockResolvedValue({ job_id: "cancelled" });
  const poll = vi.spyOn(api, "getFileJob").mockResolvedValue({
    job_id: "cancelled",
    done_s: 10,
    total_s: 90,
    finished: false,
    error: null,
    history_id: null,
  });
  const cancel = vi.spyOn(api, "cancelFileTranscription").mockResolvedValue();
  render(<FileTranscription onOpenHistory={vi.fn()} />);
  fireEvent.change(screen.getByRole("textbox", { name: "Audio file path" }), {
    target: { value: "C:/audio.wav" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Transcribe file" }));
  fireEvent.click(await screen.findByRole("button", { name: "Cancel import" }));
  await waitFor(() => expect(cancel).toHaveBeenCalledTimes(1));
  expect(screen.getByRole("button", { name: "Cancelling…" })).toBeDisabled();

  poll.mockResolvedValue({
    job_id: "cancelled",
    done_s: 10,
    total_s: 90,
    finished: true,
    error: "File transcription cancelled",
    history_id: null,
  });
  expect(await screen.findByText("Import cancelled.")).toBeVisible();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.getByRole("checkbox")).toBeEnabled();
});
