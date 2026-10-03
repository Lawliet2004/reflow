import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { api } from "../services/tauriApi";
import { FileTranscription } from "./FileTranscription";

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
