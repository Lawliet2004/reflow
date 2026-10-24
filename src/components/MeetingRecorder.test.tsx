import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { api } from "../services/tauriApi";
import { MeetingRecorder, meetingTime } from "./MeetingRecorder";
import type { Meeting } from "../types/meetings";
const meeting = (patch: Partial<Meeting> = {}): Meeting => ({
  id: "m1",
  created_at: "2026-10-24T00:00:00Z",
  config: {
    microphone: null,
    playback: "pipewire:sink.monitor",
    limit_hours: 5,
    summary_model: null,
  },
  capture: "recording",
  processing: "pending",
  elapsed_ms: 3600001,
  saved_frames: [16000, 32000],
  last_commit_ms: 3600000,
  levels: [0.4, 0.2],
  gaps: [],
  warning: null,
  error: null,
  stop_reason: null,
  processed_frames: 0,
  summary_progress: 0,
  audio_removed: false,
  ...patch,
});
beforeEach(() => {
  vi.spyOn(api, "listMeetings").mockResolvedValue([]);
  vi.spyOn(api, "getMeetingSources").mockResolvedValue({
    supported: true,
    explanation: "Devices stay pinned.",
    devices: [{ id: "mic", name: "Microphone", is_default: true, sample_rate: 16000, channels: 1 }],
    playback: [
      {
        id: "pipewire:sink.monitor",
        name: "Speakers",
        is_default: true,
        sample_rate: 48000,
        channels: 2,
      },
    ],
  });
  vi.spyOn(api, "getIntelligenceTiers").mockResolvedValue([]);
});
describe("durable meeting controls", () => {
  it("defaults to five hours and starts without a speech-model gate", async () => {
    const start = vi.spyOn(api, "startDurableMeeting").mockResolvedValue("m1");
    render(<MeetingRecorder enabled />);
    fireEvent.click(screen.getByRole("button", { name: /Meeting recordings/ }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Record meeting" })).toBeEnabled(),
    );
    expect(screen.getByLabelText("Stop after")).toHaveValue("5");
    fireEvent.click(screen.getByRole("button", { name: "Record meeting" }));
    await waitFor(() =>
      expect(start).toHaveBeenCalledWith({
        microphone: null,
        playback: "pipewire:sink.monitor",
        limit_hours: 5,
        summary_model: null,
      }),
    );
  });
  it("keeps Stop accessible while collapsed, including after settings disable meetings", async () => {
    vi.mocked(api.listMeetings).mockResolvedValue([meeting()]);
    const stop = vi.spyOn(api, "stopMeeting").mockResolvedValue();
    render(<MeetingRecorder enabled={false} />);
    const button = await screen.findByRole("button", { name: "Stop and save" });
    expect(screen.getByRole("timer")).toHaveTextContent("01:00:00 elapsed · 03:59:59 remaining");
    fireEvent.click(button);
    await waitFor(() => expect(stop).toHaveBeenCalledOnce());
  });
  it("requires explicit destructive confirmation and does not discard on Escape", async () => {
    vi.mocked(api.listMeetings).mockResolvedValue([
      meeting({ capture: "interrupted", processing: "failed", error: "Worker crashed" }),
    ]);
    const remove = vi.spyOn(api, "deleteMeeting").mockResolvedValue();
    render(<MeetingRecorder enabled />);
    fireEvent.click(screen.getByRole("button", { name: /Meeting recordings/ }));
    const del = await screen.findByRole("button", { name: "Delete meeting" });
    fireEvent.keyDown(document, { key: "Escape" });
    expect(remove).not.toHaveBeenCalled();
    fireEvent.click(del);
    expect(remove).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Delete permanently" }));
    await waitFor(() => expect(remove).toHaveBeenCalledWith("m1"));
  });
  it("disables unsupported playback and explains how to recover", async () => {
    vi.mocked(api.getMeetingSources).mockResolvedValue({
      supported: false,
      explanation: "System audio capture is not implemented on macOS.",
      devices: [],
      playback: [],
    });
    render(<MeetingRecorder enabled />);
    fireEvent.click(screen.getByRole("button", { name: /Meeting recordings/ }));
    expect(await screen.findByText(/not implemented on macOS/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Record meeting" })).toBeDisabled();
  });
  it("formats the hard five-hour boundary accurately", () => {
    expect(meetingTime(18000000)).toBe("05:00:00");
  });
});
