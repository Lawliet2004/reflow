import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import * as bridge from "../services/tauriApi";
import { NativeRuntimeSetup } from "./NativeRuntimeSetup";
import { RuntimeDownloadEvent } from "../types";

const active: RuntimeDownloadEvent = {
  attempt_id: "owned",
  revision: 20,
  phase: "downloading",
  version: "b11146",
  progress_pct: 40,
  speed_mbps: 1,
  approx_bytes: 100,
};
let emit: (event: RuntimeDownloadEvent) => void;
beforeEach(() => {
  vi.spyOn(bridge.api, "getRuntimeDownloadStatus").mockResolvedValue(null);
  vi.spyOn(bridge.api, "verifyLlamaRuntime").mockResolvedValue("generation");
  vi.spyOn(bridge.api, "installLlamaRuntime").mockResolvedValue();
  vi.spyOn(bridge.api, "cancelRuntimeDownload").mockResolvedValue();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (_, handler) => {
    emit = handler;
    return () => {};
  });
});
async function show() {
  const ready = vi.fn();
  const busy = vi.fn();
  render(
    <NativeRuntimeSetup
      settings={{ ...(await bridge.api.getSettings()), offline_mode: false }}
      onReady={ready}
      onBusyChange={busy}
      onUpdateSettings={vi.fn().mockResolvedValue(true)}
    />,
  );
  return { ready, busy };
}
it("only reports an existing runtime ready after hash and execution verification", async () => {
  const { ready } = await show();
  await screen.findByText("Native speech runtime verified.");
  expect(ready).toHaveBeenCalledOnce();
  expect(bridge.api.installLlamaRuntime).not.toHaveBeenCalled();
});
it("attaches to active setup, cancels its owned attempt and ignores stale errors", async () => {
  vi.mocked(bridge.api.getRuntimeDownloadStatus).mockResolvedValue(active);
  const { ready } = await show();
  await screen.findByRole("button", { name: "Cancel runtime setup" });
  expect(ready).not.toHaveBeenCalled();
  await waitFor(() => expect(emit).toBeDefined());
  act(() =>
    emit({ ...active, attempt_id: "old", revision: 19, phase: "error", error: "old failure" }),
  );
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Cancel runtime setup" }));
  expect(bridge.api.cancelRuntimeDownload).toHaveBeenCalledWith("owned");
  act(() => emit({ ...active, revision: 21, phase: "cancelled" }));
  expect(screen.getByRole("button", { name: "Install speech runtime" })).toBeEnabled();
  expect(ready).not.toHaveBeenCalled();
});
it("makes a failed final health check retryable rather than leaving setup busy", async () => {
  vi.mocked(bridge.api.getRuntimeDownloadStatus).mockResolvedValue(active);
  vi.mocked(bridge.api.verifyLlamaRuntime).mockRejectedValue(new Error("DLL missing"));
  const { ready } = await show();
  await screen.findByRole("button", { name: "Cancel runtime setup" });
  await waitFor(() => expect(emit).toBeDefined());
  act(() => emit({ ...active, revision: 21, phase: "complete" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("DLL missing");
  expect(screen.getByRole("button", { name: "Install speech runtime" })).toBeEnabled();
  expect(ready).not.toHaveBeenCalled();
});
