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
