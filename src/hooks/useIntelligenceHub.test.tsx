import { act, renderHook, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { api } from "../services/tauriApi";
import { useIntelligenceHub } from "./useIntelligenceHub";
import * as bridge from "../services/tauriApi";

it("polls only while a visible page consumes intelligence status", async () => {
  const metrics = await api.getSystemMetrics();
  const read = vi.spyOn(api, "getSystemMetrics").mockResolvedValue(metrics);
  vi.useFakeTimers();
  const view = renderHook(({ enabled }) => useIntelligenceHub(enabled), {
    initialProps: { enabled: false },
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(6000);
  });
  expect(read).not.toHaveBeenCalled();
  await act(async () => {
    view.rerender({ enabled: true });
  });
  expect(read).toHaveBeenCalledOnce();
  view.rerender({ enabled: false });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(6000);
  });
  expect(read).toHaveBeenCalledOnce();
});

it("retains download completion events when page polling is paused", async () => {
  const listeners = new Map<string, (payload: any) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event, handler);
    return () => {};
  });
  const status = await api.getIntelligenceStatus();
  const read = vi.spyOn(api, "getIntelligenceStatus").mockResolvedValue(status);
  const view = renderHook(() => useIntelligenceHub(false));
  await waitFor(() => expect(listeners.has("intelligence:download-progress")).toBe(true));
  expect(read).not.toHaveBeenCalled();
  await act(async () =>
    listeners.get("intelligence:download-progress")?.({
      tier: "smart_flow",
      phase: "complete",
      progress_pct: 100,
      speed_mbps: 0,
    }),
  );
  expect(read).toHaveBeenCalledOnce();
  expect(view.result.current.intelligenceDownload?.phase).toBe("complete");
});

it("pauses hidden-view polling and refreshes when the view returns", async () => {
  const metrics = await api.getSystemMetrics();
  const read = vi.spyOn(api, "getSystemMetrics").mockResolvedValue(metrics);
  const visibility = vi.spyOn(document, "visibilityState", "get");
  visibility.mockReturnValue("visible");
  const view = renderHook(useIntelligenceHub);
  await waitFor(() => expect(view.result.current.systemMetrics).toEqual(metrics));
  vi.useFakeTimers();
  const before = read.mock.calls.length;
  act(() => {
    visibility.mockReturnValue("hidden");
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(6000);
  });
  expect(read).toHaveBeenCalledTimes(before);
  await act(async () => {
    visibility.mockReturnValue("visible");
    document.dispatchEvent(new Event("visibilitychange"));
  });
  expect(read).toHaveBeenCalledTimes(before + 1);
  view.unmount();
});

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
