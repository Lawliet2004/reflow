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
  vi.spyOn(api, "setupPythonRuntime").mockResolvedValue();
  vi.spyOn(api, "installModel").mockResolvedValue();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (_event, handler) => {
    emit = handler;
    return () => {};
  });
});

async function show(offline = false, save = vi.fn().mockResolvedValue(true)) {
  const settings = await api.getSettings();
  const view = render(
    <SpeechDependencySetup
      settings={{ ...settings, offline_mode: offline }}
      onUpdateSettings={save}
    />,
  );
  await screen.findByText(/Example processor/);
  return { ...view, save };
}

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
  fireEvent.click(screen.getByRole("button", { name: "Cancel dependency setup" }));
  expect(cancel).toHaveBeenCalledOnce();
  act(() => emit({ phase: "cancelled" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Setup cancelled");
  expect(screen.getByRole("button", { name: "Retry dependency setup" })).toBeEnabled();
  expect(api.installModel).not.toHaveBeenCalled();
});

it("preserves a custom interpreter and exposes the automatic setup recovery", async () => {
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({ ...missing, overridden: true });
  await show();
  expect(screen.getByText(/Using your selected Python environment/)).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Download all dependencies" }),
  ).not.toBeInTheDocument();
});
