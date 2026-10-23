import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { api } from "../services/tauriApi";
import * as bridge from "../services/tauriApi";
import { PythonSetupProgress, SpeechSetupPlan } from "../types";
import { SpeechDependencySetup } from "./SpeechDependencySetup";

let plan: SpeechSetupPlan;
let emit: (progress: PythonSetupProgress) => void;
const missing = {
  managed_installed: false,
  receipt: null,
  overridden: false,
  setting_up: false,
  python_available: true,
  size_bytes: 0,
  torch_variant: "cpu",
};

beforeEach(async () => {
  plan = {
    hardware: await api.getCapabilities(),
    architecture: "x86_64",
    cpu_has_avx2: true,
    torch_variant: "cpu",
    acceleration: "CPU",
    explanation: "No extra graphics software is needed.",
    required_disk_bytes: 5e9,
    available_disk_bytes: 40e9,
    supported_models: ["0.6b", "1.7b", "phonon-2", "zipformer-20m"],
    blocker: null,
  };
  plan.hardware.cpu.model = "Example processor";
  vi.spyOn(api, "getSpeechSetupPlan").mockImplementation(async () => structuredClone(plan));
  vi.spyOn(api, "getPythonRuntimeStatus").mockResolvedValue(missing);
  vi.spyOn(api, "getPythonSetupProgress").mockResolvedValue(null);
  vi.spyOn(api, "setupPythonRuntime").mockResolvedValue();
  vi.spyOn(api, "verifySpeechRuntime").mockResolvedValue();
  vi.spyOn(api, "installModel").mockResolvedValue();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (_event, handler) => {
    emit = handler;
    return () => {};
  });
});

async function show(
  offline = false,
  save = vi.fn().mockResolvedValue(true),
  callbacks: {
    onReady?: () => void;
    onBusyChange?: (busy: boolean) => void;
  } = {},
) {
  const settings = await api.getSettings();
  const view = render(
    <SpeechDependencySetup
      settings={{ ...settings, offline_mode: offline }}
      onUpdateSettings={save}
      {...callbacks}
    />,
  );
  await screen.findByText(/Example processor/);
  return { ...view, save, settings: { ...settings, offline_mode: offline } };
}

it("rejoins a running setup and recovers a dropped completion without enabling downloads", async () => {
  const active: PythonSetupProgress = { attempt_id: "owned", phase: "installing_packages" };
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({
    ...missing,
    setting_up: true,
    progress: active,
  });
  vi.mocked(api.getPythonSetupProgress).mockResolvedValue(active);
  const ready = vi.fn();
  const busy = vi.fn();
  const { save } = await show(true, vi.fn(), { onReady: ready, onBusyChange: busy });
  expect(await screen.findByText(/Installing speech packages/)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Try cached dependencies" })).not.toBeInTheDocument();
  await waitFor(() => expect(api.getPythonSetupProgress).toHaveBeenCalled());
  const complete = { ...active, phase: "complete" as const };
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({
    ...missing,
    managed_installed: true,
    progress: complete,
  });
  vi.mocked(api.getPythonSetupProgress).mockResolvedValue(complete);
  expect(await screen.findByText(/Dependencies ready/, {}, { timeout: 3000 })).toBeInTheDocument();
  expect(ready).toHaveBeenCalledOnce();
  expect(busy).toHaveBeenLastCalledWith(false);
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
  expect(api.installModel).not.toHaveBeenCalled();
  expect(save).not.toHaveBeenCalled();
  const reads = vi.mocked(api.getPythonRuntimeStatus).mock.calls.length;
  // A delayed event for the completion already recovered by polling is harmless.
  await act(async () =>
    vi.mocked(bridge.safeListen).mock.calls.forEach(([, callback]) => callback(complete)),
  );
  expect(ready).toHaveBeenCalledOnce();
  expect(api.getPythonRuntimeStatus).toHaveBeenCalledTimes(reads);
});

it("reports a job that failed before reattachment without automatically retrying it", async () => {
  const active: PythonSetupProgress = { attempt_id: "owned", phase: "extracting" };
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({
    ...missing,
    setting_up: true,
    progress: active,
  });
  vi.mocked(api.getPythonSetupProgress).mockResolvedValue({
    ...active,
    phase: "error",
    error: "Stored failure",
  });
  await show();
  expect(await screen.findByRole("alert")).toHaveTextContent("Stored failure");
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
  expect(api.verifySpeechRuntime).not.toHaveBeenCalled();
});

it("follows a newly authorized setup from another surface without a second installer", async () => {
  const ready = vi.fn();
  await show(false, vi.fn(), { onReady: ready });
  const active: PythonSetupProgress = { attempt_id: "external", phase: "installing_packages" };
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({
    ...missing,
    setting_up: true,
    progress: active,
  });
  vi.mocked(api.getPythonSetupProgress).mockResolvedValue(active);
  act(() => emit(active));
  expect(await screen.findByText(/Installing speech packages/)).toBeInTheDocument();
  await waitFor(() => expect(api.getPythonSetupProgress).toHaveBeenCalled());
  act(() => emit({ ...active, phase: "verifying" }));
  expect(api.getSpeechSetupPlan).toHaveBeenCalledTimes(2);
  const complete = { ...active, phase: "complete" as const };
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({
    ...missing,
    managed_installed: true,
    progress: complete,
  });
  act(() => emit(complete));
  expect(await screen.findByText(/Dependencies ready/)).toBeInTheDocument();
  expect(ready).toHaveBeenCalledOnce();
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
});

it("joins the newer start event when a delayed status reply still says no setup is running", async () => {
  const ready = vi.fn();
  await show(false, vi.fn(), { onReady: ready });
  const active: PythonSetupProgress = {
    attempt_id: "external",
    revision: 3,
    phase: "installing_packages",
  };
  // Keep the full status reply captured before this job's start event.
  vi.mocked(api.getPythonSetupProgress).mockResolvedValue(active);
  act(() => emit(active));
  expect(await screen.findByText(/Installing speech packages/)).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Download all dependencies" }),
  ).not.toBeInTheDocument();
  const complete = { ...active, revision: 4, phase: "complete" as const };
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({
    ...missing,
    managed_installed: true,
    progress: complete,
  });
  act(() => emit(complete));
  expect(await screen.findByText(/Dependencies ready/)).toBeInTheDocument();
  expect(ready).toHaveBeenCalledOnce();
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
});

it.each([false, true])(
  "does not let a late old verification clear its replacement (failure=%s)",
  async (failure) => {
    let finish!: () => void;
    let fail!: (error: Error) => void;
    vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({
      ...missing,
      managed_installed: true,
    });
    vi.mocked(api.verifySpeechRuntime).mockImplementationOnce(
      () =>
        new Promise<void>((resolve, reject) => {
          finish = resolve;
          fail = reject;
        }),
    );
    const ready = vi.fn();
    const busy = vi.fn();
    const settings = { ...(await api.getSettings()), offline_mode: false };
    const save = vi.fn();
    const view = render(
      <SpeechDependencySetup
        settings={settings}
        onUpdateSettings={save}
        onReady={ready}
        onBusyChange={busy}
      />,
    );
    await waitFor(() => expect(api.verifySpeechRuntime).toHaveBeenCalledOnce());
    const active: PythonSetupProgress = {
      attempt_id: "replacement",
      phase: "installing_packages",
      model: "1.7b",
    };
    vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({
      ...missing,
      setting_up: true,
      progress: active,
    });
    vi.mocked(api.getPythonSetupProgress).mockResolvedValue(active);
    view.rerender(
      <SpeechDependencySetup
        settings={{ ...settings, asr: { ...settings.asr, model: "1.7b" } }}
        onUpdateSettings={save}
        onReady={ready}
        onBusyChange={busy}
      />,
    );
    expect(await screen.findByText(/Installing speech packages/)).toBeInTheDocument();
    await waitFor(() => expect(api.getPythonSetupProgress).toHaveBeenCalled());
    await act(async () => {
      if (failure) fail(new Error("Old failure"));
      else finish();
    });
    expect(ready).not.toHaveBeenCalled();
    expect(busy).toHaveBeenLastCalledWith(true);
    act(() => emit({ attempt_id: "old", phase: "complete" }));
    expect(ready).not.toHaveBeenCalled();
    const complete = { ...active, phase: "complete" as const };
    vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({
      ...missing,
      managed_installed: true,
      progress: complete,
    });
    act(() => emit(complete));
    expect(await screen.findByText(/Dependencies ready/)).toBeInTheDocument();
    expect(ready).toHaveBeenCalledOnce();
    expect(api.setupPythonRuntime).not.toHaveBeenCalled();
  },
);

it("does not claim readiness or install automatically when the verified receipt disappears", async () => {
  vi.mocked(api.getPythonRuntimeStatus)
    .mockResolvedValueOnce({ ...missing, managed_installed: true })
    .mockResolvedValue(missing);
  const ready = vi.fn();
  await show(false, vi.fn(), { onReady: ready });
  expect(await screen.findByRole("alert")).toHaveTextContent("changed during verification");
  expect(ready).not.toHaveBeenCalled();
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
});

it("shows hardware without Python probes, download claims or Python-only blockers during inspection", async () => {
  plan.blocker = "This Python profile is unsupported.";
  const settings = await api.getSettings();
  render(
    <SpeechDependencySetup
      settings={{ ...settings, asr: { ...settings.asr, runtime: "native" } }}
      inspectionOnly
      onUpdateSettings={vi.fn()}
    />,
  );
  await screen.findByText(/Example processor/);
  expect(screen.getByRole("heading", { name: "Your computer" })).toBeInTheDocument();
  expect(screen.queryByText(/private environment/)).not.toBeInTheDocument();
  expect(screen.queryByText(plan.explanation)).not.toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(api.getPythonRuntimeStatus).not.toHaveBeenCalled();
  expect(api.verifySpeechRuntime).not.toHaveBeenCalled();
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
});

it("checks hardware and installs dependencies without downloading a model", async () => {
  await show();
  expect(screen.getByText(/5.5 GB/)).toHaveTextContent("40.0 GB available");
  fireEvent.click(screen.getByRole("button", { name: "Download all dependencies" }));
  await waitFor(() => expect(api.setupPythonRuntime).toHaveBeenCalledOnce());
  expect(api.getSpeechSetupPlan).toHaveBeenCalledTimes(2);
  act(() => emit({ phase: "installing_packages", detail: "Installing speech libraries" }));
  expect(screen.getByText("Installing speech libraries")).toBeInTheDocument();
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({ ...missing, managed_installed: true });
  act(() => emit({ phase: "complete" }));
  expect(await screen.findByText(/Dependencies ready/)).toBeInTheDocument();
  expect(api.installModel).not.toHaveBeenCalled();
});

it("keeps downloads off when saving airplane mode fails", async () => {
  await show(true, vi.fn().mockResolvedValue(false));
  fireEvent.click(screen.getByRole("button", { name: "Allow downloads and install dependencies" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Downloads could not be enabled");
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
});

it("installs cached dependencies without enabling downloads", async () => {
  const { save } = await show(true);
  fireEvent.click(screen.getByRole("button", { name: "Try cached dependencies" }));
  await waitFor(() => expect(api.setupPythonRuntime).toHaveBeenCalledOnce());
  expect(save).not.toHaveBeenCalled();
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({ ...missing, managed_installed: true });
  act(() => emit({ phase: "complete" }));
  expect(await screen.findByText(/Dependencies ready/)).toBeInTheDocument();
  expect(api.verifySpeechRuntime).toHaveBeenCalledOnce();
  expect(api.installModel).not.toHaveBeenCalled();
});

it("keeps offline repair retryable when the cached pack is unavailable", async () => {
  const { save } = await show(true);
  fireEvent.click(screen.getByRole("button", { name: "Try cached dependencies" }));
  await waitFor(() => expect(api.setupPythonRuntime).toHaveBeenCalledOnce());
  act(() => emit({ phase: "error", error: "Offline mode is on" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Offline mode is on");
  expect(screen.getByRole("button", { name: "Try cached dependencies" })).toBeEnabled();
  expect(
    screen.getByRole("button", { name: "Allow downloads and install dependencies" }),
  ).toBeEnabled();
  expect(save).not.toHaveBeenCalled();
  expect(api.installModel).not.toHaveBeenCalled();
});

it("enables downloads only through the explicit online setup action", async () => {
  const { save } = await show(true);
  fireEvent.click(screen.getByRole("button", { name: "Allow downloads and install dependencies" }));
  await waitFor(() => expect(api.setupPythonRuntime).toHaveBeenCalledOnce());
  expect(save).toHaveBeenCalledExactlyOnceWith({ offline_mode: false });
  act(() => emit({ phase: "cancelled" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Setup cancelled");
});

it("blocks a large download until storage is available and permits rechecking", async () => {
  plan.blocker = "Free some space, then check again.";
  await show();
  expect(screen.getByRole("button", { name: "Download all dependencies" })).toBeDisabled();
  plan.blocker = null;
  fireEvent.click(screen.getByRole("button", { name: "Check again" }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Download all dependencies" })).toBeEnabled(),
  );
});

it("rechecks storage immediately before starting the installer", async () => {
  await show();
  plan.blocker = "There is not enough storage.";
  fireEvent.click(screen.getByRole("button", { name: "Download all dependencies" }));
  await waitFor(() => expect(api.getSpeechSetupPlan).toHaveBeenCalledTimes(2));
  expect(await screen.findByText(/Could not install dependencies/)).toHaveTextContent(
    "not enough storage",
  );
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
});

it("reports cancellation and allows retry without starting a model download", async () => {
  const cancel = vi.spyOn(api, "cancelPythonRuntimeSetup").mockResolvedValue();
  await show();
  fireEvent.click(screen.getByRole("button", { name: "Download all dependencies" }));
  await waitFor(() => expect(api.setupPythonRuntime).toHaveBeenCalledOnce());
  expect(screen.getByRole("button", { name: "Cancel dependency setup" })).toBeDisabled();
  act(() => emit({ phase: "installing_packages", attempt_id: "owned" }));
  fireEvent.click(screen.getByRole("button", { name: "Cancel dependency setup" }));
  expect(cancel).toHaveBeenCalledExactlyOnceWith("owned");
  act(() => emit({ phase: "cancelled" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Setup cancelled");
  expect(screen.getByRole("button", { name: "Retry dependency setup" })).toBeEnabled();
  expect(api.installModel).not.toHaveBeenCalled();
});

it("preserves a custom interpreter and exposes the automatic setup recovery", async () => {
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({ ...missing, overridden: true });
  vi.mocked(api.verifySpeechRuntime).mockRejectedValue(new Error("Missing torch"));
  await show();
  expect(await screen.findByText(/Using your selected Python environment/)).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Download all dependencies" }),
  ).not.toBeInTheDocument();
});

it("does not offer cancellation after verified runtime activation starts", async () => {
  const cancel = vi.spyOn(api, "cancelPythonRuntimeSetup").mockResolvedValue();
  await show();
  fireEvent.click(screen.getByRole("button", { name: "Download all dependencies" }));
  await waitFor(() => expect(api.setupPythonRuntime).toHaveBeenCalledOnce());
  act(() => emit({ phase: "activating", attempt_id: "owned-attempt" }));
  expect(screen.getByText(/Activating the speech runtime/)).toBeInTheDocument();
  const button = screen.getByRole("button", { name: "Cancel dependency setup" });
  expect(button).toBeDisabled();
  fireEvent.click(button);
  expect(cancel).not.toHaveBeenCalled();
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({ ...missing, managed_installed: true });
  act(() => emit({ phase: "complete" }));
  expect(await screen.findByText(/Dependencies ready/)).toBeInTheDocument();
});

it("does not offer cancellation while checking an already-completed setup", async () => {
  const cancel = vi.spyOn(api, "cancelPythonRuntimeSetup").mockResolvedValue();
  let finish!: () => void;
  vi.mocked(api.verifySpeechRuntime).mockImplementation(
    () =>
      new Promise<void>((resolve) => {
        finish = resolve;
      }),
  );
  await show();
  fireEvent.click(screen.getByRole("button", { name: "Download all dependencies" }));
  await waitFor(() => expect(api.setupPythonRuntime).toHaveBeenCalledOnce());
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({ ...missing, managed_installed: true });
  act(() => emit({ phase: "complete", attempt_id: "owned" }));
  await waitFor(() => expect(api.verifySpeechRuntime).toHaveBeenCalledOnce());
  const button = screen.getByRole("button", { name: "Cancel dependency setup" });
  expect(button).toBeDisabled();
  fireEvent.click(button);
  expect(cancel).not.toHaveBeenCalled();
  await act(async () => finish());
  expect(await screen.findByText(/Dependencies ready/)).toBeInTheDocument();
});

it("allows a working custom interpreter only after its libraries run", async () => {
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({ ...missing, overridden: true });
  await show();
  expect(await screen.findByText(/Dependencies ready/)).toBeInTheDocument();
  expect(api.verifySpeechRuntime).toHaveBeenCalledOnce();
  expect(api.setupPythonRuntime).not.toHaveBeenCalled();
});

it("does not trust a receipt when imports fail and exposes repair", async () => {
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({ ...missing, managed_installed: true });
  vi.mocked(api.verifySpeechRuntime).mockRejectedValue(new Error("Unusable native library"));
  await show();
  expect(await screen.findByRole("alert")).toHaveTextContent("Unusable native library");
  expect(screen.queryByText(/Dependencies ready/)).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Retry dependency setup" })).toBeEnabled();
});
