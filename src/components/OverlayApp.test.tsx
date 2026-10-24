import { act, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { OverlayApp } from "./OverlayApp";
import { api } from "../services/tauriApi";
import * as bridge from "../services/tauriApi";

it("keeps live overlay state and settings when startup snapshots arrive late", async () => {
  const initial = await api.getSettings();
  const latest = { ...initial, accent_color: "rose" as const, hud_style: "waveform" as const };
  let finishSettings!: (value: typeof initial) => void;
  let finishState!: (value: "READY") => void;
  vi.spyOn(api, "getSettings").mockImplementation(
    () =>
      new Promise((resolve) => {
        finishSettings = resolve;
      }),
  );
  vi.spyOn(api, "getAppState").mockImplementation(
    () =>
      new Promise((resolve) => {
        finishState = resolve;
      }),
  );
  const listeners = new Map<string, (value: any) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event, handler);
    return () => {};
  });
  render(<OverlayApp />);
  await waitFor(() => expect(api.getAppState).toHaveBeenCalled());
  await act(async () => {
    listeners.get("settings:changed")?.(latest);
    listeners.get("app:state-changed")?.("RECORDING");
  });
  expect(screen.getByText("Listening")).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveAttribute("data-style", "waveform");
  await act(async () => {
    finishSettings(initial);
    finishState("READY");
  });
  expect(screen.getByText("Listening")).toBeInTheDocument();
  expect(document.documentElement.dataset.accent).toBe("rose");
  expect(screen.getByRole("status")).toHaveAttribute("data-style", "waveform");
});

it("keeps active dictation visible when a previous completion is fading", async () => {
  const listeners = new Map<string, (value: any) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event, handler);
    return () => {};
  });
  render(<OverlayApp />);
  await waitFor(() => expect(listeners.has("settings:changed")).toBe(true));
  await act(async () => {
    listeners.get("app:state-changed")?.("RECORDING");
    listeners.get("transcript:final")?.({ full_text: "Hello.", audio_level: 0 });
    listeners.get("app:state-changed")?.("READY");
    listeners.get("hud:hide")?.(true);
  });
  expect(screen.getByRole("status")).toHaveAttribute("data-exiting", "true");

  await act(async () => listeners.get("app:state-changed")?.("RECORDING"));
  expect(screen.getByRole("status")).not.toHaveAttribute("data-exiting");

  // A late fade event from the previous recording must not hide active audio.
  await act(async () => listeners.get("hud:hide")?.(true));
  expect(screen.getByRole("status")).not.toHaveAttribute("data-exiting");

  await act(async () => {
    listeners.get("transcript:final")?.({ full_text: "Hello.", audio_level: 0 });
    listeners.get("app:state-changed")?.("READY");
  });
  expect(screen.getByRole("status")).toHaveAttribute("data-exiting", "true");
  await act(async () => listeners.get("hud:hide")?.(false));
  expect(screen.getByRole("status")).not.toHaveAttribute("data-exiting");

  await act(async () => listeners.get("hud:hide")?.(true));
  await act(async () => listeners.get("window:visibility")?.(true));
  expect(screen.getByRole("status")).not.toHaveAttribute("data-exiting");
});

it("records from the launcher, stops and transcribes with the cross, then returns to idle", async () => {
  const { fireEvent } = await import("@testing-library/react");
  const settings = await api.getSettings();
  vi.spyOn(api, "getSettings").mockResolvedValue({ ...settings, floating_dictation_enabled: true });
  vi.spyOn(api, "getAppState").mockResolvedValue("READY");
  const start = vi.spyOn(api, "startFloatingRecording").mockResolvedValue();
  let finishStop!: (value: string) => void;
  const stop = vi.spyOn(api, "stopRecording").mockImplementation(
    () =>
      new Promise((resolve) => {
        finishStop = resolve;
      }),
  );
  const listeners = new Map<string, (value: any) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event, handler);
    return () => {};
  });
  render(<OverlayApp />);
  fireEvent.click(await screen.findByRole("button", { name: "Start dictation" }));
  await waitFor(() => expect(start).toHaveBeenCalledExactlyOnceWith());
  await act(async () => listeners.get("app:state-changed")?.("RECORDING"));
  const cross = screen.getByRole("button", { name: "Stop recording and insert text" });
  fireEvent.click(cross);
  fireEvent.click(cross);
  expect(stop).toHaveBeenCalledTimes(1);
  expect(cross).toBeDisabled();
  await act(async () => listeners.get("app:state-changed")?.("PROCESSING"));
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  expect(screen.getByText("Transcribing")).toBeInTheDocument();
  await act(async () => {
    finishStop("Hello.");
    listeners.get("app:state-changed")?.("READY");
    listeners.get("hud:idle")?.(undefined);
  });
  expect(screen.getByRole("button", { name: "Start dictation" })).toBeEnabled();
});

it("allows retry after a recording start fails and hides the launcher when disabled", async () => {
  const { fireEvent } = await import("@testing-library/react");
  const settings = await api.getSettings();
  vi.spyOn(api, "getSettings").mockResolvedValue({ ...settings, floating_dictation_enabled: true });
  vi.spyOn(api, "getAppState").mockResolvedValue("READY");
  const start = vi
    .spyOn(api, "startFloatingRecording")
    .mockRejectedValueOnce(new Error("Microphone unavailable"))
    .mockResolvedValue();
  const listeners = new Map<string, (value: any) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event, handler);
    return () => {};
  });
  render(<OverlayApp />);
  fireEvent.click(await screen.findByRole("button", { name: "Start dictation" }));
  const retry = await screen.findByRole("button", {
    name: "Retry dictation: Microphone unavailable",
  });
  expect(retry).toBeEnabled();
  fireEvent.click(retry);
  await waitFor(() => expect(start).toHaveBeenCalledTimes(2));
  await act(async () =>
    listeners.get("settings:changed")?.({ ...settings, floating_dictation_enabled: false }),
  );
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
});

it("clears an assistant answer when the native window returns to the launcher", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "getSettings").mockResolvedValue({ ...settings, floating_dictation_enabled: true });
  vi.spyOn(api, "getAppState").mockResolvedValue("READY");
  const listeners = new Map<string, (value: any) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event, handler);
    return () => {};
  });
  render(<OverlayApp />);
  await screen.findByRole("button", { name: "Start dictation" });
  await act(async () => listeners.get("assistant:response")?.("An assistant answer."));
  expect(screen.getByText("An assistant answer.")).toBeInTheDocument();
  await act(async () => listeners.get("hud:idle")?.(undefined));
  expect(screen.queryByText("An assistant answer.")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Start dictation" })).toBeInTheDocument();
});

it("pauses and resumes one session, excludes duplicate actions, and can finish while paused", async () => {
  const { fireEvent } = await import("@testing-library/react");
  const settings = await api.getSettings();
  vi.spyOn(api, "getSettings").mockResolvedValue({
    ...settings,
    floating_dictation_enabled: true,
    recording_pause_enabled: true,
  });
  vi.spyOn(api, "getAppState").mockResolvedValue("RECORDING");
  vi.spyOn(api, "getRecordingPauseState").mockResolvedValue({ can_pause: true, paused: false });
  const start = vi.spyOn(api, "startFloatingRecording");
  const stop = vi.spyOn(api, "stopRecording").mockResolvedValue("Combined words.");
  let finishPause!: () => void;
  const pause = vi
    .spyOn(api, "setRecordingPaused")
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finishPause = resolve;
        }),
    )
    .mockResolvedValue();
  const listeners = new Map<string, (value: any) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event, handler);
    return () => {};
  });
  render(<OverlayApp />);
  const button = await screen.findByRole("button", { name: "Pause recording" });
  fireEvent.click(button);
  fireEvent.click(button);
  fireEvent.click(screen.getByRole("button", { name: "Stop recording and insert text" }));
  expect(pause).toHaveBeenCalledExactlyOnceWith(true);
  expect(stop).not.toHaveBeenCalled();
  expect(button).toBeDisabled();
  await act(async () => {
    listeners.get("recording:pause-state")?.({ can_pause: true, paused: true });
    finishPause();
  });
  expect(screen.getByText("Paused")).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Resume recording" }));
  await waitFor(() => expect(pause).toHaveBeenLastCalledWith(false));
  await act(async () =>
    listeners.get("recording:pause-state")?.({ can_pause: true, paused: false }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Pause recording" }));
  await act(async () =>
    listeners.get("recording:pause-state")?.({ can_pause: true, paused: true }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Stop recording and insert text" }));
  await waitFor(() => expect(stop).toHaveBeenCalledTimes(1));
  expect(start).not.toHaveBeenCalled();
  await act(async () => {
    listeners.get("app:state-changed")?.("READY");
    listeners.get("hud:idle")?.(undefined);
  });
  expect(screen.getByRole("button", { name: "Start dictation" })).toBeEnabled();
});

it("keeps resume available if the setting is disabled mid-pause and retries a failed control", async () => {
  const { fireEvent } = await import("@testing-library/react");
  const settings = {
    ...(await api.getSettings()),
    floating_dictation_enabled: true,
    recording_pause_enabled: true,
  };
  vi.spyOn(api, "getSettings").mockResolvedValue(settings);
  vi.spyOn(api, "getAppState").mockResolvedValue("RECORDING");
  vi.spyOn(api, "getRecordingPauseState").mockResolvedValue({ can_pause: true, paused: true });
  const pause = vi
    .spyOn(api, "setRecordingPaused")
    .mockRejectedValueOnce(new Error("Could not resume. Try again."))
    .mockResolvedValue();
  const listeners = new Map<string, (value: any) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event, handler);
    return () => {};
  });
  render(<OverlayApp />);
  await screen.findByRole("button", { name: "Resume recording" });
  await act(async () =>
    listeners.get("settings:changed")?.({ ...settings, recording_pause_enabled: false }),
  );
  fireEvent.click(screen.getByRole("button", { name: "Resume recording" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not resume. Try again.");
  expect(screen.getByRole("button", { name: "Resume recording" })).toBeEnabled();
  fireEvent.click(screen.getByRole("button", { name: "Resume recording" }));
  await waitFor(() => expect(pause).toHaveBeenCalledTimes(2));
  await act(async () =>
    listeners.get("recording:pause-state")?.({ can_pause: true, paused: false }),
  );
  expect(screen.queryByRole("button", { name: "Pause recording" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Stop recording and insert text" })).toBeEnabled();
});

it("does not overwrite a live pause event with a late startup snapshot", async () => {
  vi.spyOn(api, "getAppState").mockResolvedValue("RECORDING");
  let resolveSnapshot!: (value: { can_pause: boolean; paused: boolean }) => void;
  vi.spyOn(api, "getRecordingPauseState").mockImplementation(
    () =>
      new Promise((resolve) => {
        resolveSnapshot = resolve;
      }),
  );
  const listeners = new Map<string, (value: any) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event, handler);
    return () => {};
  });
  render(<OverlayApp />);
  await waitFor(() => expect(api.getRecordingPauseState).toHaveBeenCalled());
  await act(async () => {
    listeners.get("recording:pause-state")?.({ can_pause: true, paused: true });
    resolveSnapshot({ can_pause: true, paused: false });
  });
  expect(screen.getByRole("button", { name: "Resume recording" })).toBeEnabled();
  expect(screen.getByText("Paused")).toBeVisible();
});

it("hides microphone pause for meeting and external recording sessions", async () => {
  vi.spyOn(api, "getAppState").mockResolvedValue("RECORDING");
  vi.spyOn(api, "getRecordingPauseState").mockResolvedValue({ can_pause: false, paused: false });
  render(<OverlayApp />);
  await screen.findByRole("button", { name: "Stop recording and insert text" });
  expect(screen.queryByRole("button", { name: "Pause recording" })).not.toBeInTheDocument();
});
