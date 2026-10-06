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
  fireEvent.click(screen.getByRole("button", { name: "Download and use Phonon-2 · Fast English" }));
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
  expect(screen.getByRole("status")).toHaveTextContent(
    "English only. Choose English or Auto-detect",
  );
  expect(screen.queryByRole("button", { name: /4-bit/ })).not.toBeInTheDocument();
  expect(screen.getByRole("combobox", { name: "Speech runtime" })).toBeDisabled();
});

it("selects Zipformer with its Python runtime and automatic precision", async () => {
  const install = vi.spyOn(api, "installModel").mockResolvedValue();
  vi.spyOn(api, "getModelStatus").mockResolvedValue({
    ...(await api.getModelStatus()),
    installed: false,
  });
  const { save } = await modelPage(false, undefined, { runtime: "native", precision: "int4" });
  fireEvent.click(screen.getByRole("button", { name: "Download and use Zipformer 20M INT8" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith({
      preset: "custom",
      asr: expect.objectContaining({
        model: "zipformer-20m",
        runtime: "python",
        precision: "auto",
      }),
    }),
  );
  await waitFor(() => expect(install).toHaveBeenCalledExactlyOnceWith("zipformer-20m"));
});

it("shows Zipformer's English restriction and hides precision controls", async () => {
  await modelPage(true, undefined, { model: "zipformer-20m" });
  expect(screen.getByRole("status")).toHaveTextContent(
    "English only. Choose English or Auto-detect",
  );
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
  const row = selection.closest("li")!;
  fireEvent.click(within(row).getByRole("button", { name: "Download" }));
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
  fireEvent.click(screen.getByRole("button", { name: "Download and use 1.7B · Higher accuracy" }));
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
  fireEvent.click(screen.getByRole("button", { name: "Download and use 0.6B · Faster" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith({
      preset: "custom",
      asr: expect.objectContaining({ model: "0.6b", precision: "int4" }),
    }),
  );
});

it("deletes a non-selected downloaded speech model after confirming", async () => {
  vi.spyOn(api, "getDownloadedModels").mockResolvedValue([
    {
      id: "0.6b",
      kind: "asr",
      label: "0.6B",
      size_bytes: 1_565_000_000,
      active: true,
      partial: false,
    },
    {
      id: "1.7b",
      kind: "asr",
      label: "1.7B",
      size_bytes: 4_076_000_000,
      active: false,
      partial: false,
    },
    {
      id: "smart_flow",
      kind: "llm",
      label: "Qwen3.5 0.8B",
      size_bytes: 532_517_120,
      active: true,
      partial: false,
    },
  ]);
  const removeAsr = vi.spyOn(api, "removeModel").mockResolvedValue();
  const removeLlm = vi.spyOn(api, "removeIntelligenceModel").mockResolvedValue();
  await modelPage();
  const row = (await screen.findByText("1.7B")).closest("li")!;
  fireEvent.click(within(row).getByRole("button", { name: "Delete 1.7B" }));
  fireEvent.click(within(row).getByRole("button", { name: "Delete" }));
  await waitFor(() => expect(removeAsr).toHaveBeenCalledExactlyOnceWith("1.7b"));
  expect(removeLlm).not.toHaveBeenCalled();
});

it("deletes an installed speech model from its library row", async () => {
  const getDownloaded = vi.spyOn(api, "getDownloadedModels").mockResolvedValue([
    {
      id: "1.7b",
      kind: "asr",
      label: "1.7B",
      size_bytes: 4_076_000_000,
      active: false,
      partial: false,
    },
  ]);
  const removeAsr = vi.spyOn(api, "removeModel").mockResolvedValue();
  await modelPage();
  const deleteButton = await screen.findByRole("button", {
    name: "Delete 1.7B · Higher accuracy",
  });
  const libraryRow = deleteButton.closest("li")!;
  fireEvent.click(deleteButton);
  fireEvent.click(within(libraryRow).getByRole("button", { name: "Delete" }));
  await waitFor(() => expect(removeAsr).toHaveBeenCalledExactlyOnceWith("1.7b"));
  await waitFor(() => expect(getDownloaded.mock.calls.length).toBeGreaterThanOrEqual(2));
});

it("switches to an installed model without downloading", async () => {
  vi.spyOn(api, "getDownloadedModels").mockResolvedValue([
    {
      id: "1.7b",
      kind: "asr",
      label: "1.7B",
      size_bytes: 4_076_000_000,
      active: false,
      partial: false,
    },
  ]);
  const status = await api.getModelStatus();
  vi.spyOn(api, "getModelStatus").mockResolvedValue({ ...status, installed: true });
  const reload = vi.spyOn(api, "reloadModel").mockResolvedValue();
  const install = vi.spyOn(api, "installModel").mockResolvedValue();
  const { save } = await modelPage();
  fireEvent.click(await screen.findByRole("button", { name: "Use 1.7B · Higher accuracy" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith({
      preset: "custom",
      asr: expect.objectContaining({ model: "1.7b" }),
    }),
  );
  await waitFor(() => expect(reload).toHaveBeenCalled());
  expect(install).not.toHaveBeenCalled();
});
