import { act, fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import type { IntelligenceHub } from "../hooks/useIntelligenceHub";
import { api } from "../services/tauriApi";
import { SettingsView } from "./SettingsView";

const intelligence: IntelligenceHub = {
  flowStatus: null,
  systemMetrics: null,
  intelligenceTiers: null,
  capabilitiesRefreshKey: 0,
  intelligenceDownload: null,
  activeDownloadTiers: new Set(),
  runtimeDownload: null,
  runtimeDownloadActive: false,
  runtimeDownloadError: null,
  refresh: async () => {},
  notifyToast: () => {},
  lastToast: null,
  refreshCapabilities: () => {},
};

it("restores the selected page and search focus after clearing an unmatched search", async () => {
  render(
    <SettingsView
      settings={await api.getSettings()}
      onUpdateSettings={vi.fn().mockResolvedValue(true)}
      modelStatus={null}
      onReloadModel={vi.fn()}
      intelligence={intelligence}
      onInstallRuntime={vi.fn()}
      onRemoveRuntime={vi.fn()}
    />,
  );
  const search = screen.getByRole("textbox", { name: "Search settings" });
  fireEvent.change(search, { target: { value: "no such setting" } });
  expect(screen.getByRole("heading", { name: "No results" })).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Clear settings search" }));
  expect(screen.getByRole("heading", { name: "General" })).toBeVisible();
  expect(search).toHaveValue("");
  expect(search).toHaveFocus();
});

it.each(["false", "reject"])(
  "blocks speech changes and microphone checks until persistence settles, then reports a %s failure",
  async (failure) => {
    const settings = await api.getSettings();
    let resolve!: (saved: boolean) => void;
    let reject!: (error: Error) => void;
    const update = vi.fn(
      () =>
        new Promise<boolean>((yes, no) => {
          resolve = yes;
          reject = no;
        }),
    );
    const microphone = vi.spyOn(api, "testMicrophone");
    const recognition = vi.spyOn(api, "testRecognition");
    render(
      <SettingsView
        settings={settings}
        onUpdateSettings={update}
        modelStatus={null}
        onReloadModel={vi.fn()}
        intelligence={intelligence}
        onInstallRuntime={vi.fn()}
        onRemoveRuntime={vi.fn()}
      />,
    );
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Speech" }));
    });
    fireEvent.change(screen.getByRole("combobox", { name: "Dictation language" }), {
      target: { value: "ja" },
    });

    expect(screen.getByRole("combobox", { name: "Dictation language" })).toBeDisabled();
    expect(screen.getByRole("slider", { name: "Input gain" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Test microphone" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Test recognition" })).toBeDisabled();
    expect(screen.getByText(/Saving/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Test microphone" }));
    fireEvent.click(screen.getByRole("button", { name: "Test recognition" }));
    expect(microphone).not.toHaveBeenCalled();
    expect(recognition).not.toHaveBeenCalled();

    await act(async () => {
      if (failure === "false") resolve(false);
      else reject(new Error("Disk unavailable"));
    });
    expect(await screen.findByRole("alert")).toHaveTextContent(/save|saved/i);
    expect(screen.getByRole("button", { name: "Test microphone" })).toBeEnabled();
    expect(screen.queryByText(/Saving/)).not.toBeInTheDocument();
  },
);
