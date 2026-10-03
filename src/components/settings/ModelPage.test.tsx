import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { ModelPage } from "./ModelPage";
import { api } from "../../services/tauriApi";
import type { IntelligenceHub } from "../../hooks/useIntelligenceHub";
import type { AsrSettings } from "../../types";

async function modelPage(
  installed = true,
  save = vi.fn().mockResolvedValue(true),
  asr: Partial<AsrSettings> = {},
) {
  const settings = await api.getSettings();
  const status = await api.getModelStatus();
  const intelligence: IntelligenceHub = {
    flowStatus: null,
    systemMetrics: null,
    intelligenceTiers: [],
    capabilitiesRefreshKey: 0,
    intelligenceDownload: null,
    activeDownloadTiers: new Set(),
    runtimeDownload: null,
    runtimeDownloadActive: false,
    runtimeDownloadError: null,
    refresh: vi.fn(),
    notifyToast: vi.fn(),
    lastToast: null,
    refreshCapabilities: vi.fn(),
  };
  render(
    <ModelPage
      settings={{
        ...settings,
        preset: "custom",
        asr: { ...settings.asr, model: "0.6b", precision: "auto", runtime: "python", ...asr },
      }}
      modelStatus={{
        ...status,
        installed,
        loaded: installed,
        backend: installed ? "CPU" : "Not loaded",
      }}
      intelligence={intelligence}
      onUpdateSettings={save}
      onReloadModel={vi.fn()}
      intelligenceDownload={null}
      activeDownloadTiers={new Set()}
      runtimeDownload={null}
      runtimeDownloadActive={false}
      runtimeDownloadError={null}
      onInstallRuntime={vi.fn()}
      onRemoveRuntime={vi.fn()}
    />,
  );
  await waitFor(() =>
    expect(screen.getByRole("combobox", { name: "Compute backend" })).toBeInTheDocument(),
  );
  return { save, intelligence };
}

it("offers installation for the selected missing speech model", async () => {
  const install = vi.spyOn(api, "installModel").mockResolvedValue();
  await modelPage(false);
  expect(screen.getByText("Speech model is not installed")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Download speech model" }));
  await waitFor(() => expect(install).toHaveBeenCalledExactlyOnceWith("0.6b"));
});

it("does not reload or download a model after a failed settings save", async () => {
  const save = vi.fn().mockResolvedValue(false);
  const reload = vi.spyOn(api, "reloadModel").mockResolvedValue();
  const install = vi.spyOn(api, "installModel").mockResolvedValue();
  await modelPage(true, save);
  fireEvent.click(screen.getByRole("button", { name: /1.7B · Higher accuracy/ }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(1));
  expect(reload).not.toHaveBeenCalled();
  expect(install).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: /8-bit/ }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
  expect(reload).not.toHaveBeenCalled();
});

it("uses the persisted CUDA device value", async () => {
  const { save } = await modelPage();
  fireEvent.change(screen.getByRole("combobox", { name: "Compute backend" }), {
    target: { value: "cuda" },
  });
  expect(save).toHaveBeenLastCalledWith({
    preset: "custom",
    asr: expect.objectContaining({ device: "cuda" }),
  });
});

it("saves and reloads 4-bit precision for the 0.6B model", async () => {
  const reload = vi.spyOn(api, "reloadModel").mockResolvedValue();
  const { save } = await modelPage();
  const fourBit = screen.getByRole("button", { name: /4-bit/ });
  expect(fourBit).toBeEnabled();
  fireEvent.click(fourBit);
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith({
      preset: "custom",
      asr: expect.objectContaining({ model: "0.6b", precision: "int4" }),
    }),
  );
  await waitFor(() => expect(reload).toHaveBeenCalledTimes(1));
});

it("preserves 4-bit precision when switching from 1.7B to 0.6B", async () => {
  vi.spyOn(api, "installModel").mockResolvedValue();
  const { save } = await modelPage(true, undefined, { model: "1.7b", precision: "int4" });
  fireEvent.click(screen.getByRole("button", { name: /0.6B · Faster/ }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith({
      preset: "custom",
      asr: expect.objectContaining({ model: "0.6b", precision: "int4" }),
    }),
  );
});
