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
