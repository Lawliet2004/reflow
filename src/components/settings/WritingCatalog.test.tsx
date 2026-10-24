import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { WritingCatalog } from "./WritingCatalog";
import { api } from "../../services/tauriApi";
import type { IntelligenceHub } from "../../hooks/useIntelligenceHub";

const hub: IntelligenceHub = {
  flowStatus: null,
  systemMetrics: null,
  intelligenceTiers: [],
  capabilitiesRefreshKey: 0,
  intelligenceDownload: null,
  activeDownloadTiers: new Set(),
  runtimeDownload: null,
  runtimeDownloadActive: false,
  runtimeDownloadError: null,
  refresh: async () => {},
  notifyToast: vi.fn(),
  lastToast: null,
  refreshCapabilities: vi.fn(),
};

it("selects a supported named writing model without downloading it", async () => {
  const save = vi.fn().mockResolvedValue(true);
  const install = vi.spyOn(api, "installIntelligenceModel").mockResolvedValue();
  render(
    <WritingCatalog
      settings={{ ...(await api.getSettings()), offline_mode: false }}
      intelligence={hub}
      onUpdateSettings={save}
      onInstallRuntime={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Use MiniCPM5 2B" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith(expect.objectContaining({ flow_model: "minicpm5-2b" })),
  );
  expect(install).not.toHaveBeenCalled();
  expect(screen.queryByText("Preparing download…")).not.toBeInTheDocument();
});

it("keeps named downloads separate from model choice and reports failures", async () => {
  const save = vi.fn().mockResolvedValue(true);
  const failure = new Error("Disk is full");
  const install = vi.spyOn(api, "installIntelligenceModel").mockRejectedValue(failure);
  const notifyToast = vi.fn();
  render(
    <WritingCatalog
      settings={{ ...(await api.getSettings()), offline_mode: false }}
      intelligence={{ ...hub, notifyToast }}
      onUpdateSettings={save}
      onInstallRuntime={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Download Qwen3.5 4B · Q4_K_M" }));
  await waitFor(() => expect(install).toHaveBeenCalledWith("qwen3.5-4b"));
  await waitFor(() => expect(notifyToast).toHaveBeenCalledWith("error", "Error: Disk is full"));
  expect(save).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Download Qwen3.5 4B · Q4_K_M" })).toBeEnabled();
});

it("saves a summary preference independently from dictation polish", async () => {
  const save = vi.fn().mockResolvedValue(true);
  render(
    <WritingCatalog
      settings={await api.getSettings()}
      intelligence={hub}
      onUpdateSettings={save}
      onInstallRuntime={vi.fn()}
    />,
  );
  fireEvent.change(screen.getByRole("combobox", { name: "Meeting notes model" }), {
    target: { value: "gemma4-e4b" },
  });
  await waitFor(() =>
    expect(save).toHaveBeenCalledExactlyOnceWith({ summary_model: "gemma4-e4b" }),
  );
});

it("uses a model for summaries when browsing meeting notes without changing text polish", async () => {
  const save = vi.fn().mockResolvedValue(true);
  render(
    <WritingCatalog
      task="notes"
      settings={await api.getSettings()}
      intelligence={hub}
      onUpdateSettings={save}
      onInstallRuntime={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Use Gemma 4 E4B · Q4_K_M for notes" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledExactlyOnceWith({ summary_model: "gemma4-e4b" }),
  );
});
