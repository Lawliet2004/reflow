import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { GeneralPage } from "./GeneralPage";
import { api } from "../../services/tauriApi";
import * as bridge from "../../services/tauriApi";

const wayland = {
  session: "wayland",
  assigned_shortcuts: {},
  shortcut: { state: "ready" as const, assigned_trigger: "Ctrl+Shift+Space", message: null },
  keyboard: { state: "ready" as const, message: null },
  hud_supported: true,
  foreground_app_available: false,
};

it("captures a KDE shortcut inside Reflow and shows only the confirmed binding", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "getDesktopIntegrationStatus").mockResolvedValue({
    ...wayland,
    shortcut_editable: true,
  });
  let finish!: (value: typeof wayland) => void;
  const assign = vi.spyOn(api, "setDesktopShortcut").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const configure = vi.spyOn(api, "configureGlobalShortcuts").mockResolvedValue();
  const update = vi.fn();
  render(<GeneralPage settings={settings} onUpdateSettings={update} />);
  const key = await screen.findByRole("button", {
    name: "Change dictation shortcut: Ctrl+Shift+Space",
  });
  fireEvent.click(key);
  await waitFor(() => expect(key).toHaveTextContent("Press keys"));
  expect(key).toHaveTextContent("Press keys");
  fireEvent.keyDown(window, { key: " ", code: "Space", ctrlKey: true });
  expect(assign).toHaveBeenCalledExactlyOnceWith("dictation", "Ctrl+Space");
  expect(key).toBeDisabled();
  expect(key).toHaveTextContent("Ctrl+Shift+Space");
  await act(async () =>
    finish({
      ...wayland,
      assigned_shortcuts: { dictate: "Ctrl+Space" },
      shortcut: { ...wayland.shortcut, assigned_trigger: "Ctrl+Space" },
    }),
  );
  expect(
    screen.getByRole("button", { name: "Change dictation shortcut: Ctrl+Space" }),
  ).toBeEnabled();
  expect(update).not.toHaveBeenCalled();
  expect(configure).not.toHaveBeenCalled();
});

it("keeps the previous KDE binding when a captured shortcut conflicts", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "getDesktopIntegrationStatus").mockResolvedValue({
    ...wayland,
    shortcut_editable: true,
  });
  vi.spyOn(api, "setDesktopShortcut").mockRejectedValue(new Error("That shortcut is already used"));
  render(<GeneralPage settings={settings} onUpdateSettings={vi.fn()} />);
  fireEvent.click(
    await screen.findByRole("button", { name: "Change dictation shortcut: Ctrl+Shift+Space" }),
  );
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Change dictation shortcut: Ctrl+Shift+Space" }),
    ).toHaveTextContent("Press keys"),
  );
  fireEvent.keyDown(window, { key: "t", code: "KeyT", ctrlKey: true, altKey: true });
  expect(await screen.findByRole("alert")).toHaveTextContent("That shortcut is already used");
  expect(
    screen.getByRole("button", { name: "Change dictation shortcut: Ctrl+Shift+Space" }),
  ).toBeEnabled();
});

it("waits for portal approval before capturing a KDE key", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "getDesktopIntegrationStatus").mockResolvedValue({
    ...wayland,
    shortcut_editable: true,
    shortcut: { state: "pending", assigned_trigger: null, message: "Approve Reflow shortcuts" },
  });
  render(<GeneralPage settings={settings} onUpdateSettings={vi.fn()} />);
  expect(await screen.findByRole("button", { name: "Assign dictation shortcut" })).toBeDisabled();
  expect(screen.getByText("Approve Reflow shortcuts")).toBeInTheDocument();
});

it("updates displayed keys when the desktop changes their assignment", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "getDesktopIntegrationStatus").mockResolvedValue(wayland);
  let changed!: (value: typeof wayland) => void;
  vi.spyOn(bridge, "safeListen").mockImplementation(async (_event, handler) => {
    changed = handler;
    return () => {};
  });
  const update = vi.fn();
  render(<GeneralPage settings={settings} onUpdateSettings={update} />);
  await screen.findByRole("button", { name: "Change dictation shortcut: Ctrl+Shift+Space" });
  await act(async () =>
    changed({
      ...wayland,
      assigned_shortcuts: { dictation: "Meta+F8", cancel: "Ctrl+Alt+Backspace" },
      shortcut: { ...wayland.shortcut, assigned_trigger: "Meta+F8" },
    }),
  );
  expect(screen.getByRole("button", { name: "Change dictation shortcut: Meta+F8" })).toBeEnabled();
  expect(
    screen.getByRole("button", { name: "Change cancel shortcut: Ctrl+Alt+Backspace" }),
  ).toBeEnabled();
  expect(
    screen.queryByRole("button", { name: "Change dictation shortcut: Ctrl+Shift+Space" }),
  ).not.toBeInTheDocument();
  expect(update).not.toHaveBeenCalled();
});

it("shows the desktop's pending shortcut approval separately from speech readiness", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "getDesktopIntegrationStatus").mockResolvedValue({
    ...wayland,
    shortcut: {
      state: "pending",
      assigned_trigger: null,
      message: "Allow Reflow's global shortcuts in the desktop permission dialog.",
    },
  });
  render(<GeneralPage settings={settings} onUpdateSettings={vi.fn()} />);
  expect(await screen.findByText(/Allow Reflow/)).toHaveTextContent("desktop permission dialog");
  expect(screen.getByRole("button", { name: "Assign dictation shortcut" })).toBeEnabled();
});

it("explains when native Wayland shortcut registration is ready", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "getDesktopIntegrationStatus").mockResolvedValue(wayland);
  render(<GeneralPage settings={settings} onUpdateSettings={vi.fn()} />);
  expect(await screen.findByText(/Dictation shortcut ready/)).toHaveTextContent("Ctrl+Shift+Space");
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("opens the actual desktop binding without saving a misleading local shortcut", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "getDesktopIntegrationStatus").mockResolvedValue(wayland);
  const configure = vi.spyOn(api, "configureGlobalShortcuts").mockResolvedValue();
  const update = vi.fn();
  render(<GeneralPage settings={settings} onUpdateSettings={update} />);
  const key = await screen.findByRole("button", {
    name: "Change dictation shortcut: Ctrl+Shift+Space",
  });
  fireEvent.click(key);
  expect(configure).toHaveBeenCalledOnce();
  expect(update).not.toHaveBeenCalled();
});

it("reports shortcut status read failures", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "getDesktopIntegrationStatus").mockRejectedValue(new Error("portal unavailable"));
  render(<GeneralPage settings={settings} onUpdateSettings={vi.fn()} />);
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not check desktop integration");
});

it("offers native desktop configuration and reports a failed request", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "getDesktopIntegrationStatus").mockResolvedValue({
    ...wayland,
    shortcut: {
      state: "unassigned",
      assigned_trigger: null,
      message: "No dictation key is assigned by your Wayland desktop.",
    },
  });
  const configure = vi
    .spyOn(api, "configureGlobalShortcuts")
    .mockRejectedValue(new Error("session closed"));
  render(<GeneralPage settings={settings} onUpdateSettings={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: "Configure desktop shortcuts" }));
  expect(await screen.findByText(/Could not open desktop shortcuts/)).toHaveTextContent(
    "session closed",
  );
  expect(configure).toHaveBeenCalledOnce();
});

it("waits for the desktop settings launch and allows retry after a launch failure", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "getDesktopIntegrationStatus").mockResolvedValue(wayland);
  let failLaunch!: (error: Error) => void;
  vi.spyOn(api, "configureGlobalShortcuts").mockImplementationOnce(
    () => new Promise<void>((_, reject) => (failLaunch = reject)),
  );
  render(<GeneralPage settings={settings} onUpdateSettings={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: "Configure desktop shortcuts" }));
  expect(screen.getByRole("button", { name: "Opening desktop shortcuts…" })).toBeDisabled();
  failLaunch(new Error("Could not launch KDE shortcut settings"));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Could not launch KDE shortcut settings",
  );
  expect(screen.getByRole("button", { name: "Configure desktop shortcuts" })).toBeEnabled();
});

it("lets the floating dictation button be switched off and on", async () => {
  const settings = await api.getSettings();
  const update = vi.fn();
  const { rerender } = render(
    <GeneralPage
      settings={{ ...settings, floating_dictation_enabled: true }}
      onUpdateSettings={update}
    />,
  );
  fireEvent.click(screen.getByRole("switch", { name: "Floating dictation button" }));
  expect(update).toHaveBeenLastCalledWith({ floating_dictation_enabled: false });
  rerender(
    <GeneralPage
      settings={{ ...settings, floating_dictation_enabled: false }}
      onUpdateSettings={update}
    />,
  );
  fireEvent.click(screen.getByRole("switch", { name: "Floating dictation button" }));
  expect(update).toHaveBeenLastCalledWith({ floating_dictation_enabled: true });
});

it("shows pause recording by default and saves the toggle", async () => {
  const settings = await api.getSettings();
  const update = vi.fn();
  const { rerender } = render(<GeneralPage settings={settings} onUpdateSettings={update} />);
  const toggle = screen.getByRole("switch", { name: "Pause recording button" });
  expect(toggle).toHaveAttribute("aria-checked", "true");
  fireEvent.click(toggle);
  expect(update).toHaveBeenLastCalledWith({ recording_pause_enabled: false });
  rerender(
    <GeneralPage
      settings={{ ...settings, recording_pause_enabled: false }}
      onUpdateSettings={update}
    />,
  );
  fireEvent.click(screen.getByRole("switch", { name: "Pause recording button" }));
  expect(update).toHaveBeenLastCalledWith({ recording_pause_enabled: true });
});
