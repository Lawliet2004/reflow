import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, it, expect, vi } from "vitest";
import { PythonRuntimeSetup } from "./PythonRuntimeSetup";
import { api } from "../services/tauriApi";
import * as bridge from "../services/tauriApi";
import { PythonRuntimeStatus, PythonSetupProgress } from "../types";

const baseStatus: PythonRuntimeStatus = {
  managed_installed: false,
  receipt: null,
  overridden: false,
  setting_up: false,
  python_available: false,
  torch_variant: "cpu",
  size_bytes: 0,
};

afterEach(() => vi.useRealTimers());

async function props(overrides: Partial<PythonRuntimeStatus> = {}) {
  const settings = await api.getSettings();
  vi.spyOn(api, "getPythonRuntimeStatus").mockResolvedValue({ ...baseStatus, ...overrides });
  const listeners = new Map<string, (payload: any) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event, handler);
    return () => {};
  });
  const emit = (payload: PythonSetupProgress) =>
    listeners.get("python-runtime:setup-progress")?.(payload);
  return { settings, emit, save: vi.fn().mockResolvedValue(true) };
}

it("offers one-click setup when no usable interpreter exists", async () => {
  const { settings, save } = await props({ python_available: false });
  const setup = vi.spyOn(api, "setupPythonRuntime").mockResolvedValue();
  render(
    <PythonRuntimeSetup settings={{ ...settings, offline_mode: false }} onUpdateSettings={save} />,
  );
  const button = await screen.findByRole("button", { name: /Set up speech runtime/ });
  fireEvent.click(button);
  await waitFor(() => expect(setup).toHaveBeenCalledOnce());
});

it("asks for download consent when airplane mode is on", async () => {
  const { settings, save } = await props({ python_available: false });
  const setup = vi.spyOn(api, "setupPythonRuntime").mockResolvedValue();
  render(
    <PythonRuntimeSetup settings={{ ...settings, offline_mode: true }} onUpdateSettings={save} />,
  );
  const allow = await screen.findByRole("button", { name: "Allow downloads" });
  fireEvent.click(allow);
  await waitFor(() => expect(save).toHaveBeenCalledWith({ offline_mode: false }));
  await waitFor(() => expect(setup).toHaveBeenCalledOnce());
});

it("can retry cached dependencies while keeping airplane mode enabled", async () => {
  const { settings, save } = await props();
  const setup = vi.spyOn(api, "setupPythonRuntime").mockResolvedValue();
  render(
    <PythonRuntimeSetup settings={{ ...settings, offline_mode: true }} onUpdateSettings={save} />,
  );
  fireEvent.click(await screen.findByRole("button", { name: "Try cached dependencies" }));
  await waitFor(() => expect(setup).toHaveBeenCalledOnce());
  expect(save).not.toHaveBeenCalled();
});

it("shows phase, download progress and Cancel while setting up", async () => {
  const { settings, emit } = await props({ setting_up: true });
  const cancel = vi.spyOn(api, "cancelPythonRuntimeSetup").mockResolvedValue();
  render(
    <PythonRuntimeSetup
      settings={{ ...settings, offline_mode: false }}
      onUpdateSettings={vi.fn()}
    />,
  );
  await act(async () => emit({ phase: "downloading", progress_pct: 42, attempt_id: "owned" }));
  expect(await screen.findByText(/Downloading Python/)).toBeInTheDocument();
  await act(async () =>
    emit({
      phase: "installing_packages",
      detail: "Collecting transformers==5.15.1",
      attempt_id: "owned",
    }),
  );
  expect(screen.getByText("Collecting transformers==5.15.1")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(cancel).toHaveBeenCalledExactlyOnceWith("owned"));
});

it("surfaces an error and retries", async () => {
  const { settings, emit } = await props({ setting_up: true });
  const setup = vi.spyOn(api, "setupPythonRuntime").mockResolvedValue();
  render(
    <PythonRuntimeSetup
      settings={{ ...settings, offline_mode: false }}
      onUpdateSettings={vi.fn()}
    />,
  );
  await act(async () => emit({ phase: "error", error: "Download failed: HTTP 404" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("HTTP 404");
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(setup).toHaveBeenCalledOnce());
});

it("restores activation from the status snapshot and honors terminal events over an older snapshot", async () => {
  const { settings, emit } = await props({
    managed_installed: true,
    setting_up: true,
    progress: { phase: "activating", attempt_id: "committing" },
  });
  const cancel = vi.spyOn(api, "cancelPythonRuntimeSetup").mockResolvedValue();
  render(<PythonRuntimeSetup settings={settings} onUpdateSettings={vi.fn()} showRemove />);
  expect(await screen.findByText(/Activating the speech runtime/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
  expect(screen.queryByRole("button", { name: /Remove speech runtime/ })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(cancel).not.toHaveBeenCalled();
  await act(async () => emit({ phase: "complete", attempt_id: "committing" }));
  expect(screen.queryByRole("button", { name: "Cancel" })).not.toBeInTheDocument();
});

it("reports the configured interpreter instead of offering setup", async () => {
  const { settings } = await props({ overridden: true, python_available: false });
  const modelStatus = await api.getModelStatus();
  render(
    <PythonRuntimeSetup
      settings={settings}
      modelStatus={{ ...modelStatus, failure_kind: "python_missing" }}
      onUpdateSettings={vi.fn()}
    />,
  );
  expect(await screen.findByText(/Using your own Python/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: /Set up speech runtime/ })).not.toBeInTheDocument();
});

it("orders snapshots and events across retries before enabling cancellation", async () => {
  const { settings, emit } = await props({
    setting_up: true,
    progress: { phase: "activating", attempt_id: "first", revision: 3 },
  });
  const cancel = vi.spyOn(api, "cancelPythonRuntimeSetup").mockResolvedValue();
  render(<PythonRuntimeSetup settings={settings} onUpdateSettings={vi.fn()} />);
  expect(await screen.findByText(/Activating the speech runtime/)).toBeInTheDocument();
  await act(async () => emit({ phase: "verifying", attempt_id: "first", revision: 2 }));
  expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
  await act(async () => emit({ phase: "downloading", attempt_id: "retry", revision: 5 }));
  await act(async () => emit({ phase: "complete", attempt_id: "first", revision: 4 }));
  expect(screen.getByText(/Downloading Python/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(cancel).toHaveBeenCalledExactlyOnceWith("retry"));
});

it("recovers a lost completion after an IPC interruption without restarting setup", async () => {
  vi.useFakeTimers();
  const active = { attempt_id: "owned", revision: 1, phase: "verifying" as const };
  const { settings } = await props({ setting_up: true, progress: active });
  const setup = vi.spyOn(api, "setupPythonRuntime").mockResolvedValue();
  const readProgress = vi
    .spyOn(api, "getPythonSetupProgress")
    .mockRejectedValueOnce(new Error("IPC interrupted"));
  await act(async () => {
    render(<PythonRuntimeSetup settings={settings} onUpdateSettings={vi.fn()} />);
  });
  const initialStatusReads = vi.mocked(api.getPythonRuntimeStatus).mock.calls.length;
  await act(async () => vi.advanceTimersByTimeAsync(1000));
  expect(api.getPythonRuntimeStatus).toHaveBeenCalledTimes(initialStatusReads);
  expect(screen.getByText(/Checking the speech runtime/)).toBeInTheDocument();
  const complete = { ...active, phase: "complete" as const, revision: 2 };
  readProgress.mockResolvedValue(complete);
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({
    ...baseStatus,
    managed_installed: true,
    progress: complete,
  });
  await act(async () => vi.advanceTimersByTimeAsync(1000));
  expect(screen.getByText(/Managed speech runtime installed/)).toBeInTheDocument();
  expect(setup).not.toHaveBeenCalled();
  expect(vi.getTimerCount()).toBe(0);
});

it("ignores a pending progress reply after settings detach", async () => {
  vi.useFakeTimers();
  const active = { attempt_id: "owned", revision: 1, phase: "verifying" as const };
  const { settings } = await props({ setting_up: true, progress: active });
  let reply!: (value: PythonSetupProgress) => void;
  const readProgress = vi.spyOn(api, "getPythonSetupProgress").mockImplementation(
    () =>
      new Promise((resolve) => {
        reply = resolve;
      }),
  );
  let view!: ReturnType<typeof render>;
  await act(async () => {
    view = render(<PythonRuntimeSetup settings={settings} onUpdateSettings={vi.fn()} />);
  });
  await act(async () => vi.advanceTimersByTimeAsync(1000));
  const reads = vi.mocked(api.getPythonRuntimeStatus).mock.calls.length;
  view.unmount();
  await act(async () => reply({ ...active, phase: "complete", revision: 2 }));
  await vi.advanceTimersByTimeAsync(2000);
  expect(api.getPythonRuntimeStatus).toHaveBeenCalledTimes(reads);
  expect(readProgress).toHaveBeenCalledOnce();
  expect(vi.getTimerCount()).toBe(0);
});

it("keeps an acknowledged retry busy until its fresh owner snapshot arrives", async () => {
  const { settings } = await props({
    progress: { phase: "error", attempt_id: "first", revision: 4, error: "Stored failure" },
  });
  let acknowledge!: () => void;
  const setup = vi.spyOn(api, "setupPythonRuntime").mockImplementation(
    () =>
      new Promise((resolve) => {
        acknowledge = resolve;
      }),
  );
  render(<PythonRuntimeSetup settings={settings} onUpdateSettings={vi.fn()} />);
  fireEvent.click(await screen.findByRole("button", { name: "Retry" }));
  expect(screen.getByText(/Downloading Python/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
  let snapshot!: (value: PythonRuntimeStatus) => void;
  vi.mocked(api.getPythonRuntimeStatus).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        snapshot = resolve;
      }),
  );
  await act(async () => acknowledge());
  expect(setup).toHaveBeenCalledOnce();
  expect(screen.queryByRole("button", { name: "Retry" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
  await act(async () =>
    snapshot({
      ...baseStatus,
      setting_up: true,
      progress: { phase: "extracting", attempt_id: "retry", revision: 5 },
    }),
  );
  expect(screen.getByText(/Extracting Python/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
});

it("shows the installed receipt and offers removal", async () => {
  const { settings } = await props({
    managed_installed: true,
    receipt: {
      pbs_tag: "20261003",
      python_version: "3.12.15",
      torch_variant: "cpu",
      created_at: "2026-10-14T00:00:00Z",
    },
    python_available: true,
    torch_variant: "cpu",
    size_bytes: 2_600_000_000,
  });
  render(
    <PythonRuntimeSetup
      settings={{ ...settings, offline_mode: false }}
      onUpdateSettings={vi.fn()}
      showRemove
    />,
  );
  expect(await screen.findByText(/Python 3\.12\.15 · torch cpu/)).toBeInTheDocument();
  const remove = screen.getByRole("button", { name: /Remove speech runtime/ });
  expect(remove).toHaveTextContent("2.6 GB");
  const removeCall = vi.spyOn(api, "removePythonRuntime").mockResolvedValue(2_600_000_000);
  fireEvent.click(remove);
  await waitFor(() => expect(removeCall).toHaveBeenCalledOnce());
});

it("offers dependency setup even when system Python exists without a managed receipt", async () => {
  const { settings } = await props({ python_available: true });
  render(
    <PythonRuntimeSetup
      settings={{ ...settings, offline_mode: false }}
      onUpdateSettings={vi.fn()}
    />,
  );
  await waitFor(() => expect(api.getPythonRuntimeStatus).toHaveBeenCalled());
  expect(await screen.findByRole("button", { name: "Set up speech runtime" })).toBeInTheDocument();
});

it("offers repair when a managed environment has missing speech dependencies", async () => {
  const { settings } = await props({ managed_installed: true, python_available: true });
  const status = await api.getModelStatus();
  render(
    <PythonRuntimeSetup
      settings={{ ...settings, offline_mode: false }}
      modelStatus={{ ...status, failure_kind: "missing_dependency" }}
      onUpdateSettings={vi.fn()}
    />,
  );
  expect(await screen.findByRole("button", { name: "Set up speech runtime" })).toBeInTheDocument();
  expect(screen.queryByText(/Managed speech runtime installed/)).not.toBeInTheDocument();
});
