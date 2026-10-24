import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api, safeListen } from "./tauriApi";
import { prepareSpeechRuntime } from "./speechSetup";
import { PythonRuntimeStatus, PythonSetupProgress } from "../types";

vi.mock("./tauriApi", () => ({
  api: {
    getPythonRuntimeStatus: vi.fn(),
    getPythonSetupProgress: vi.fn(),
    setupPythonRuntime: vi.fn(),
    verifySpeechRuntime: vi.fn(),
    activateCachedSpeechRuntime: vi.fn().mockResolvedValue(false),
  },
  safeListen: vi.fn(),
}));

beforeEach(() => vi.clearAllMocks());
afterEach(() => vi.useRealTimers());

const empty: PythonRuntimeStatus = {
  managed_installed: false,
  receipt: null,
  overridden: false,
  setting_up: false,
  python_available: true, // System Python alone does not provide speech packages.
  size_bytes: 0,
  torch_variant: "cpu",
};

function runtime(status: Partial<PythonRuntimeStatus> = {}) {
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({ ...empty, ...status });
  vi.mocked(api.getPythonSetupProgress).mockResolvedValue(status.progress ?? null);
  vi.mocked(api.setupPythonRuntime).mockResolvedValue();
  vi.mocked(api.verifySpeechRuntime).mockResolvedValue();
  let emit!: (value: PythonSetupProgress) => void;
  const unsubscribe = vi.fn();
  vi.mocked(safeListen).mockImplementation(async (_event, handler) => {
    emit = handler;
    return unsubscribe;
  });
  return { emit: (value: PythonSetupProgress) => emit(value), unsubscribe };
}

it("waits for verified runtime completion rather than the command acknowledgement", async () => {
  const { emit, unsubscribe } = runtime();
  const done = vi.fn();
  const progress = vi.fn();
  const task = prepareSpeechRuntime(new AbortController().signal, progress).then(done);
  await vi.waitFor(() => expect(api.setupPythonRuntime).toHaveBeenCalledOnce());
  expect(done).not.toHaveBeenCalled();
  emit({ phase: "installing_packages" });
  expect(progress).toHaveBeenCalledWith({ phase: "installing_packages" });
  emit({ phase: "complete" });
  await task;
  expect(done).toHaveBeenCalledOnce();
  expect(unsubscribe).toHaveBeenCalledOnce();
});

it("polls only the cheap owned snapshot to recover a lost completion event", async () => {
  vi.useFakeTimers();
  const active = { attempt_id: "owned", phase: "installing_packages" as const };
  const { unsubscribe } = runtime({ setting_up: true, progress: active });
  const progress = vi.fn();
  const task = prepareSpeechRuntime(new AbortController().signal, progress, null, "owned");
  await vi.waitFor(() => expect(progress).toHaveBeenCalledWith(active));
  const initialStatusReads = vi.mocked(api.getPythonRuntimeStatus).mock.calls.length;
  const initialProgressReads = vi.mocked(api.getPythonSetupProgress).mock.calls.length;
  await vi.advanceTimersByTimeAsync(1000);
  expect(api.getPythonSetupProgress).toHaveBeenCalledTimes(initialProgressReads + 1);
  expect(api.getPythonRuntimeStatus).toHaveBeenCalledTimes(initialStatusReads);
  const complete = { ...active, phase: "complete" as const };
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({
    ...empty,
    managed_installed: true,
    progress: complete,
  });
  vi.mocked(api.getPythonSetupProgress).mockResolvedValue(complete);
  await vi.advanceTimersByTimeAsync(1000);
  await task;
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
  expect(api.verifySpeechRuntime).toHaveBeenCalledOnce();
  expect(unsubscribe).toHaveBeenCalledOnce();
  expect(vi.getTimerCount()).toBe(0);
});

it("does not restart a captured job that failed or disappeared before joining", async () => {
  const active = { attempt_id: "owned", phase: "installing_packages" as const };
  runtime({ setting_up: true, progress: active });
  vi.mocked(api.getPythonSetupProgress).mockResolvedValue({
    ...active,
    phase: "error",
    error: "Stored failure",
  });
  await expect(
    prepareSpeechRuntime(new AbortController().signal, vi.fn(), null, "owned"),
  ).rejects.toThrow("Stored failure");
  vi.mocked(api.getPythonSetupProgress).mockResolvedValue(null);
  await expect(
    prepareSpeechRuntime(new AbortController().signal, vi.fn(), null, "owned"),
  ).rejects.toThrow("ended or changed");
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
  expect(api.verifySpeechRuntime).not.toHaveBeenCalled();
});

it("keeps newer activation events when a delayed snapshot and older events arrive", async () => {
  vi.useFakeTimers();
  const active = { attempt_id: "owned", revision: 2, phase: "verifying" as const };
  const { emit, unsubscribe } = runtime({ setting_up: true, progress: active });
  const progress = vi.fn();
  const task = prepareSpeechRuntime(new AbortController().signal, progress, null, "owned");
  await vi.waitFor(() => expect(progress).toHaveBeenCalledWith(active));
  let reply!: (value: PythonSetupProgress) => void;
  vi.mocked(api.getPythonSetupProgress).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        reply = resolve;
      }),
  );
  await vi.advanceTimersByTimeAsync(1000);
  const activating = { ...active, revision: 3, phase: "activating" as const };
  emit(activating);
  reply(active);
  await Promise.resolve();
  emit(active);
  emit({ attempt_id: "owned", phase: "extracting" });
  expect(progress).toHaveBeenCalledTimes(2);
  expect(progress).toHaveBeenLastCalledWith(activating);
  vi.mocked(api.getPythonSetupProgress).mockResolvedValue({
    attempt_id: "previous",
    revision: 1,
    phase: "complete",
  });
  await vi.advanceTimersByTimeAsync(1000);
  expect(progress).toHaveBeenCalledTimes(2);
  const complete = { ...active, revision: 4, phase: "complete" as const };
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({
    ...empty,
    managed_installed: true,
    progress: complete,
  });
  emit(complete);
  await task;
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
  expect(api.verifySpeechRuntime).toHaveBeenCalledOnce();
  expect(unsubscribe).toHaveBeenCalledOnce();
  expect(vi.getTimerCount()).toBe(0);
});

it("resumes a job that completed before joining without another download", async () => {
  runtime({ managed_installed: true, progress: { attempt_id: "owned", phase: "complete" } });
  await prepareSpeechRuntime(new AbortController().signal, vi.fn(), null, "owned");
  expect(api.verifySpeechRuntime).toHaveBeenCalledOnce();
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
});

it("recovers through snapshots when event subscription and one poll fail", async () => {
  vi.useFakeTimers();
  const active = { attempt_id: "owned", phase: "verifying" as const };
  runtime({ setting_up: true, progress: active });
  vi.mocked(safeListen).mockRejectedValueOnce(new Error("Event bridge interrupted"));
  const progress = vi.fn();
  const task = prepareSpeechRuntime(new AbortController().signal, progress, null, "owned");
  await vi.waitFor(() => expect(progress).toHaveBeenCalledWith(active));
  vi.mocked(api.getPythonSetupProgress).mockRejectedValueOnce(new Error("IPC interrupted"));
  await vi.advanceTimersByTimeAsync(1000);
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
  const complete = { ...active, phase: "complete" as const };
  vi.mocked(api.getPythonSetupProgress).mockResolvedValue(complete);
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({
    ...empty,
    managed_installed: true,
    progress: complete,
  });
  await vi.advanceTimersByTimeAsync(1000);
  await task;
  expect(api.verifySpeechRuntime).toHaveBeenCalledOnce();
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
  expect(vi.getTimerCount()).toBe(0);
});

it("rejects a replacement snapshot and detaches without claiming the new job", async () => {
  vi.useFakeTimers();
  const active = { attempt_id: "owned", phase: "extracting" as const };
  const { unsubscribe } = runtime({ setting_up: true, progress: active });
  const progress = vi.fn();
  const task = prepareSpeechRuntime(new AbortController().signal, progress, null, "owned");
  const rejection = expect(task).rejects.toThrow("ended or changed");
  await vi.waitFor(() => expect(progress).toHaveBeenCalledWith(active));
  vi.mocked(api.getPythonSetupProgress).mockResolvedValue({
    attempt_id: "replacement",
    phase: "complete",
  });
  await vi.advanceTimersByTimeAsync(1000);
  await rejection;
  expect(unsubscribe).toHaveBeenCalledOnce();
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
  expect(api.verifySpeechRuntime).not.toHaveBeenCalled();
  expect(vi.getTimerCount()).toBe(0);
});

it("clears snapshot polling when its view detaches", async () => {
  vi.useFakeTimers();
  const active = { attempt_id: "owned", phase: "downloading" as const };
  const { unsubscribe } = runtime({ setting_up: true, progress: active });
  const abort = new AbortController();
  const progress = vi.fn();
  const task = prepareSpeechRuntime(abort.signal, progress, null, "owned");
  const rejection = expect(task).rejects.toThrow("Setup stopped");
  await vi.waitFor(() => expect(progress).toHaveBeenCalledWith(active));
  const beforeDetach = vi.mocked(api.getPythonSetupProgress).mock.calls.length;
  abort.abort();
  await rejection;
  await vi.advanceTimersByTimeAsync(3000);
  expect(api.getPythonSetupProgress).toHaveBeenCalledTimes(beforeDetach);
  expect(unsubscribe).toHaveBeenCalledOnce();
  expect(vi.getTimerCount()).toBe(0);
});

it.each(["error", "cancelled"] as const)("stops the sequence on %s", async (phase) => {
  const { emit, unsubscribe } = runtime();
  const task = prepareSpeechRuntime(new AbortController().signal, vi.fn());
  const rejection = expect(task).rejects.toThrow(phase === "error" ? "Disk full" : "cancelled");
  await vi.waitFor(() => expect(api.setupPythonRuntime).toHaveBeenCalledOnce());
  emit({ phase, error: "Disk full" });
  await rejection;
  expect(unsubscribe).toHaveBeenCalledOnce();
});

it("reuses a managed runtime without downloading it again", async () => {
  runtime({ managed_installed: true });
  await prepareSpeechRuntime(new AbortController().signal, vi.fn());
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
});

it("preserves a custom interpreter and explains how to recover when it is broken", async () => {
  runtime({ overridden: true });
  await prepareSpeechRuntime(new AbortController().signal, vi.fn());
  vi.mocked(api.verifySpeechRuntime).mockRejectedValue(new Error("Missing torch"));
  await expect(
    prepareSpeechRuntime(new AbortController().signal, vi.fn(), "missing_dependency"),
  ).rejects.toThrow("Clear Python interpreter");
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
});

it("rejects a receipt whose native libraries no longer run", async () => {
  runtime({ managed_installed: true });
  vi.mocked(api.verifySpeechRuntime).mockRejectedValue(new Error("Broken native library"));
  await expect(prepareSpeechRuntime(new AbortController().signal, vi.fn())).rejects.toThrow(
    "Broken native library",
  );
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
});

it("joins an existing setup without starting another installer", async () => {
  const { emit } = runtime({ setting_up: true });
  const progress = vi.fn();
  const task = prepareSpeechRuntime(new AbortController().signal, progress);
  await vi.waitFor(() => expect(progress).toHaveBeenCalled());
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
  emit({ phase: "complete" });
  await task;
});

it("detaches on unmount so completing setup cannot download a model afterwards", async () => {
  const { emit, unsubscribe } = runtime();
  const controller = new AbortController();
  const progress = vi.fn();
  const task = prepareSpeechRuntime(controller.signal, progress);
  const rejection = expect(task).rejects.toThrow("Setup stopped");
  await vi.waitFor(() => expect(api.setupPythonRuntime).toHaveBeenCalledOnce());
  controller.abort();
  await rejection;
  expect(unsubscribe).toHaveBeenCalledOnce();
  emit({ phase: "complete" });
  expect(progress).not.toHaveBeenCalledWith({ phase: "complete" });
});

it("waits for a running repair even when an earlier installation has a receipt", async () => {
  const { emit } = runtime({ managed_installed: true, setting_up: true });
  const done = vi.fn();
  const task = prepareSpeechRuntime(new AbortController().signal, vi.fn()).then(done);
  await vi.waitFor(() => expect(safeListen).toHaveBeenCalled());
  expect(done).not.toHaveBeenCalled();
  emit({ phase: "complete" });
  await task;
  expect(done).toHaveBeenCalledOnce();
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
});

it("joins a setup started concurrently by another surface", async () => {
  const { emit } = runtime();
  vi.mocked(api.getPythonRuntimeStatus)
    .mockResolvedValueOnce(empty)
    .mockResolvedValueOnce(empty)
    .mockResolvedValue({ ...empty, setting_up: true });
  vi.mocked(api.setupPythonRuntime).mockRejectedValue(new Error("Already in progress"));
  const progress = vi.fn();
  const task = prepareSpeechRuntime(new AbortController().signal, progress);
  await vi.waitFor(() => expect(api.getPythonRuntimeStatus).toHaveBeenCalledTimes(3));
  emit({ phase: "complete" });
  await task;
  expect(progress).toHaveBeenCalledWith({ phase: "complete" });
});

it("ignores a previous attempt's completion after joining its replacement", async () => {
  const active = {
    attempt_id: "replacement",
    model: "0.6b",
    device: "cpu",
    phase: "verifying" as const,
  };
  const { emit } = runtime({ setting_up: true, progress: active });
  const done = vi.fn();
  const progress = vi.fn();
  const task = prepareSpeechRuntime(new AbortController().signal, progress).then(done);
  await vi.waitFor(() => expect(progress).toHaveBeenCalledWith(active));
  emit({ attempt_id: "previous", phase: "complete" });
  await Promise.resolve();
  expect(done).not.toHaveBeenCalled();
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({
    ...empty,
    managed_installed: true,
    progress: { ...active, phase: "complete" },
  });
  emit({ ...active, phase: "complete" });
  await task;
  expect(done).toHaveBeenCalledOnce();
  expect(progress).not.toHaveBeenCalledWith({ attempt_id: "previous", phase: "complete" });
});

it.each(["error", "cancelled"] as const)(
  "recovers a missed terminal %s from the backend snapshot without restarting setup",
  async (phase) => {
    const active = { attempt_id: "owned", phase: "installing_packages" as const };
    runtime();
    vi.mocked(api.getPythonRuntimeStatus)
      .mockResolvedValueOnce({ ...empty, setting_up: true, progress: active })
      .mockResolvedValue({ ...empty, progress: { ...active, phase, error: "Stored failure" } });
    await expect(prepareSpeechRuntime(new AbortController().signal, vi.fn())).rejects.toThrow(
      phase === "error" ? "Stored failure" : "cancelled",
    );
    expect(api.setupPythonRuntime).not.toHaveBeenCalled();
  },
);

it("reuses a verified cached runtime without downloading packages", async () => {
  runtime();
  vi.mocked(api.activateCachedSpeechRuntime).mockResolvedValueOnce(true);
  await prepareSpeechRuntime(new AbortController().signal, vi.fn());
  expect(api.verifySpeechRuntime).toHaveBeenCalledOnce();
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
});
