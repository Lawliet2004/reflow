import { beforeEach, expect, it, vi } from "vitest";
import { api, safeListen } from "./tauriApi";
import { prepareSpeechRuntime } from "./speechSetup";
import { PythonRuntimeStatus, PythonSetupProgress } from "../types";

vi.mock("./tauriApi", () => ({
  api: { getPythonRuntimeStatus: vi.fn(), setupPythonRuntime: vi.fn() },
  safeListen: vi.fn(),
}));

beforeEach(() => vi.clearAllMocks());

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
  vi.mocked(api.setupPythonRuntime).mockResolvedValue();
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
  await expect(
    prepareSpeechRuntime(new AbortController().signal, vi.fn(), "missing_dependency"),
  ).rejects.toThrow("Clear Python interpreter");
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
