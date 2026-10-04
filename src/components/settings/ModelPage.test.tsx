import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { ModelPage } from "./ModelPage";
import { api } from "../../services/tauriApi";
import type { IntelligenceHub } from "../../hooks/useIntelligenceHub";
import type { AsrSettings } from "../../types";

it("selects Phonon with its Python runtime and automatic precision", async () => {
  const install = vi.spyOn(api, "installModel").mockResolvedValue();
  vi.spyOn(api, "getModelStatus").mockResolvedValue({
    ...(await api.getModelStatus()),
    installed: false,
  });
  const { save } = await modelPage(false, undefined, { runtime: "native", precision: "int4" });
  fireEvent.click(screen.getByRole("button", { name: /Phonon-2/ }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith({
      preset: "custom",
      asr: expect.objectContaining({ model: "phonon-2", runtime: "python", precision: "auto" }),
    }),
  );
  await waitFor(() => expect(install).toHaveBeenCalledExactlyOnceWith("phonon-2"));
});

it("shows Phonon's English restriction and hides Qwen-only settings", async () => {
  await modelPage(true, undefined, { model: "phonon-2" });
  expect(screen.getByText(/English only/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /4-bit/ })).not.toBeInTheDocument();
  expect(screen.getByRole("combobox", { name: "Speech runtime" })).toBeDisabled();
});

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

it("keeps downloading separate from choosing the model", async () => {
  const install = vi.spyOn(api, "installIntelligenceModel").mockResolvedValue();
  const { save } = await modelPage();
  const selection = screen.getByRole("button", { name: "Use Qwen3.5 2B" });
  const card = selection.parentElement!;
  fireEvent.click(within(card).getByRole("button", { name: "Download" }));
  await waitFor(() => expect(install).toHaveBeenCalledExactlyOnceWith("deep_context"));
  expect(save).not.toHaveBeenCalled();
  expect(selection).toHaveAttribute("aria-pressed", "false");
  fireEvent.click(selection);
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ flow_model: "qwen3.5-2b" })),
  );
});

it.each([
  ["Use Qwen3.5 0.8B", "smart_flow", "qwen3.5-0.8b"],
  ["Use Qwen3.5 2B", "deep_context", "qwen3.5-2b"],
  ["Use no LLM", "raw_verbatim", "none"],
])("selects %s from its card without starting downloads", async (name, tier, model) => {
  const install = vi.spyOn(api, "installIntelligenceModel").mockResolvedValue();
  const { save } = await modelPage();
  fireEvent.click(screen.getByRole("button", { name }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({ intelligence_tier: tier, flow_model: model }),
    ),
  );
  expect(install).not.toHaveBeenCalled();
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
