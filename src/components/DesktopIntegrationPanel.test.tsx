import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import * as bridge from "../services/tauriApi";
import { DesktopIntegrationStatus } from "../types";
import { DesktopIntegrationPanel } from "./DesktopIntegrationPanel";

const denied: DesktopIntegrationStatus = {
  session: "wayland",
  assigned_shortcuts: {},
  shortcut: { state: "unavailable", assigned_trigger: null, message: "Portal restarted" },
  keyboard: { state: "denied", message: "Keyboard permission was cancelled" },
  hud_supported: true,
  foreground_app_available: false,
};

it("offers recovery after keyboard denial and portal restart", async () => {
  vi.spyOn(bridge.api, "getDesktopIntegrationStatus").mockResolvedValue(denied);
  const keyboard = vi.spyOn(bridge.api, "requestKeyboardAccess").mockResolvedValue();
  const reconnect = vi.spyOn(bridge.api, "retryDesktopShortcuts").mockResolvedValue();
  render(<DesktopIntegrationPanel />);
  fireEvent.click(await screen.findByRole("button", { name: "Retry keyboard access" }));
  await waitFor(() => expect(keyboard).toHaveBeenCalledOnce());
  fireEvent.click(screen.getByRole("button", { name: "Reconnect shortcuts" }));
  await waitFor(() => expect(reconnect).toHaveBeenCalledOnce());
  expect(screen.getByText(/Foreground app details are unavailable/)).toBeInTheDocument();
});

it("keeps a newer permission event when an older snapshot resolves late", async () => {
  let changed!: (value: DesktopIntegrationStatus) => void;
  vi.spyOn(bridge, "safeListen").mockImplementation(async (_event, handler) => {
    changed = handler;
    return () => {};
  });
  let resolve!: (value: DesktopIntegrationStatus) => void;
  const read = vi.spyOn(bridge.api, "getDesktopIntegrationStatus").mockImplementation(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  render(<DesktopIntegrationPanel />);
  await waitFor(() => expect(read).toHaveBeenCalledOnce());
  await act(async () => {
    changed({ ...denied, keyboard: { state: "ready", message: null } });
    resolve(denied);
  });
  expect(screen.getByText("Keyboard access ready for automatic paste.")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Retry keyboard access" })).not.toBeInTheDocument();
});
