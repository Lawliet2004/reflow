import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { it, expect, vi } from "vitest";
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
    <PythonRuntimeSetup
      settings={{ ...settings, offline_mode: false }}
      onUpdateSettings={save}
    />,
  );
  const button = await screen.findByRole("button", { name: /Set up speech runtime/ });
  fireEvent.click(button);
  await waitFor(() => expect(setup).toHaveBeenCalledOnce());
});

it("asks for download consent when airplane mode is on", async () => {
  const { settings, save } = await props({ python_available: false });
  const setup = vi.spyOn(api, "setupPythonRuntime").mockResolvedValue();
  render(
    <PythonRuntimeSetup
      settings={{ ...settings, offline_mode: true }}
      onUpdateSettings={save}
    />,
  );
  const allow = await screen.findByRole("button", { name: "Allow downloads" });
  fireEvent.click(allow);
  await waitFor(() => expect(save).toHaveBeenCalledWith({ offline_mode: false }));
  await waitFor(() => expect(setup).toHaveBeenCalledOnce());
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
  await act(async () => emit({ phase: "downloading", progress_pct: 42 }));
  expect(await screen.findByText(/Downloading Python/)).toBeInTheDocument();
  await act(async () =>
    emit({ phase: "installing_packages", detail: "Collecting transformers==5.15.1" }),
  );
  expect(screen.getByText("Collecting transformers==5.15.1")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  await waitFor(() => expect(cancel).toHaveBeenCalledOnce());
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
  expect(
    screen.queryByRole("button", { name: /Set up speech runtime/ }),
  ).not.toBeInTheDocument();
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

it("renders nothing when a working interpreter is already configured", async () => {
  const { settings } = await props({ python_available: true });
  const { container } = render(
    <PythonRuntimeSetup
      settings={{ ...settings, offline_mode: false }}
      onUpdateSettings={vi.fn()}
    />,
  );
  await waitFor(() => expect(api.getPythonRuntimeStatus).toHaveBeenCalled());
  expect(container).toBeEmptyDOMElement();
});
