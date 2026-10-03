import { act, renderHook, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { api } from "../services/tauriApi";
import { useIntelligenceHub } from "./useIntelligenceHub";
import * as bridge from "../services/tauriApi";

it("reports a failed intelligence subscription while keeping runtime events active", async () => {
  const listeners = new Map<string, (payload: any) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    if (event === "intelligence:download-progress") throw new Error("Listener unavailable");
    listeners.set(event, handler);
    return () => {};
  });
  const { result } = renderHook(useIntelligenceHub);
  await waitFor(() => expect(result.current.lastToast?.text).toContain("Listener unavailable"));
  expect(listeners.has("runtime:download-progress")).toBe(true);
  await act(async () =>
    listeners.get("runtime:download-progress")?.({
      phase: "error",
      error: "Runtime download failed",
    }),
  );
  expect(result.current.lastToast).toEqual({ kind: "error", text: "Runtime download failed" });
});

it("applies healthy status independently and coalesces overlapping refreshes", async () => {
  let release!: () => void;
  const blocked = new Promise<void>((resolve) => {
    release = resolve;
  });
  const flow = vi.spyOn(api, "getIntelligenceStatus").mockImplementation(async () => {
    await blocked;
    throw new Error("Unavailable");
  });
  const metrics = await api.getSystemMetrics();
  vi.spyOn(api, "getSystemMetrics").mockResolvedValue(metrics);
  const tiers = await api.getIntelligenceTiers();
  vi.spyOn(api, "getIntelligenceTiers").mockResolvedValue(tiers);
  const { result } = renderHook(useIntelligenceHub);
  await waitFor(() => expect(result.current.systemMetrics).toEqual(metrics));
  expect(result.current.intelligenceTiers).toEqual(tiers);
  await act(async () => {
    void result.current.refresh();
    void result.current.refresh();
  });
  expect(flow).toHaveBeenCalledOnce();
  await act(async () => {
    release();
    await result.current.refresh();
  });
  expect(result.current.flowStatus).toBeNull();
});
