import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { CleanupPage } from "./CleanupPage";
import { api } from "../../services/tauriApi";
import type { IntelligenceHub } from "../../hooks/useIntelligenceHub";

async function renderPage() {
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
    <CleanupPage
      settings={await api.getSettings()}
      intelligence={intelligence}
      onUpdateSettings={vi.fn().mockResolvedValue(true)}
      modelStatus={null}
      intelligenceDownload={null}
      activeDownloadTiers={new Set()}
      runtimeDownload={null}
      runtimeDownloadActive={false}
      runtimeDownloadError={null}
      onInstallRuntime={vi.fn()}
      onRemoveRuntime={vi.fn()}
    />,
  );
}

it("reports rule cleanup when the requested AI edit is rejected", async () => {
  vi.spyOn(api, "previewTierCleanup").mockResolvedValue({
    text: "Keep src/api.ts.",
    latency_ms: 35,
    tier_used: "smart_flow",
    model_used: "qwen3.5-0.8b",
    rewriter_used: false,
    rewriter_error: "Unsafe cleanup result",
  });
  await renderPage();
  fireEvent.click(screen.getByRole("button", { name: "Test" }));
  expect(await screen.findByText("Keep src/api.ts.")).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("Unsafe cleanup result");
  expect(screen.getByText(/Basic cleanup · 35ms/)).toBeInTheDocument();
  expect(screen.queryByText(/with qwen3.5-0.8b/)).not.toBeInTheDocument();
});

it("attributes output to the model only when it actually rewrote", async () => {
  vi.spyOn(api, "previewTierCleanup").mockResolvedValue({
    text: "We could ship Thursday.",
    latency_ms: 600,
    tier_used: "smart_flow",
    model_used: "qwen3.5-0.8b",
    rewriter_used: true,
  });
  await renderPage();
  fireEvent.click(screen.getByRole("button", { name: "Test" }));
  expect(await screen.findByText(/Cleaned in 600ms with qwen3.5-0.8b/)).toBeInTheDocument();
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});
