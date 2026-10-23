import { useState } from "react";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { api } from "../services/tauriApi";
import * as bridge from "../services/tauriApi";
import {
  AppSettings,
  MicrophoneHealth,
  ModelStatus,
  PythonSetupProgress,
  SpeechSetupPlan,
} from "../types";
import {
  readSetupCheckpoint,
  writeSetupCheckpoint,
  microphoneConfiguration,
  shortcutConfiguration,
  speechConfiguration,
} from "../services/onboarding";
import { Onboarding } from "./Onboarding";

let listeners: Map<string, Set<(payload: any) => void>>;
let plan: SpeechSetupPlan;
const healthy: MicrophoneHealth = {
  device: "USB microphone",
  duration_ms: 3000,
  peak: 0.5,
  rms: 0.1,
  clipped_pct: 0,
  dropped_chunks: 0,
  assessment: "Healthy",
};
const runtime = {
  managed_installed: true,
  receipt: null,
  overridden: false,
  setting_up: false,
  python_available: true,
  size_bytes: 0,
  torch_variant: "cpu",
};
const emit = (event: string, payload: unknown) =>
  act(() => listeners.get(event)?.forEach((handler) => handler(payload)));

beforeEach(async () => {
  vi.spyOn(api, "verifySpeechRuntime").mockResolvedValue();
  localStorage.removeItem("reflow.onboarding.v1");
  localStorage.removeItem("reflow.onboarding.completed");
  listeners = new Map();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    const set = listeners.get(event) ?? new Set();
    set.add(handler);
    listeners.set(event, set);
    return () => {
      set.delete(handler);
    };
  });
  vi.spyOn(api, "getAudioDevices").mockResolvedValue([
    { id: "usb", name: "USB microphone", is_default: true, sample_rate: 48000, channels: 1 },
  ]);
  vi.spyOn(api, "testMicrophone").mockResolvedValue(healthy);
  vi.spyOn(api, "recommendedAsrModel").mockResolvedValue("1.7b");
  vi.spyOn(api, "getPythonRuntimeStatus").mockResolvedValue(runtime);
  vi.spyOn(api, "beginShortcutCapture").mockResolvedValue(71);
  vi.spyOn(api, "endShortcutCapture").mockResolvedValue();
  vi.spyOn(api, "verifyLlamaRuntime").mockResolvedValue("verified-generation");
  vi.spyOn(api, "installModel").mockResolvedValue();
  plan = {
    hardware: await api.getCapabilities(),
    architecture: "x86_64",
    cpu_has_avx2: true,
    torch_variant: "cpu",
    acceleration: "CPU",
    explanation: "CPU speech is available.",
    required_disk_bytes: 5e9,
    available_disk_bytes: 40e9,
    supported_models: ["0.6b", "1.7b", "phonon-2", "zipformer-20m"],
    blocker: null,
  };
  vi.spyOn(api, "getSpeechSetupPlan").mockImplementation(async () => structuredClone(plan));
});

function Harness({
  initial,
  status,
  save,
  complete,
}: {
  initial: AppSettings;
  status: ModelStatus;
  save: (changes: Partial<AppSettings>) => Promise<boolean>;
  complete: (outcome: "completed" | "deferred") => void;
}) {
  const [settings, setSettings] = useState(initial);
  return (
    <Onboarding
      settings={settings}
      modelStatus={status}
      onComplete={complete}
      onUpdateSettings={async (changes) => {
        const ok = await save(changes);
        if (ok) setSettings((old) => ({ ...old, ...changes }));
        return ok;
      }}
    />
  );
}
async function show(
  options: {
    initial?: AppSettings;
    ready?: boolean;
    save?: (changes: Partial<AppSettings>) => Promise<boolean>;
  } = {},
) {
  const initial = options.initial ?? { ...(await api.getSettings()), offline_mode: false };
  const base = await api.getModelStatus();
  const status = {
    ...base,
    installed: options.ready ?? false,
    loaded: options.ready ?? false,
    is_loading: false,
    is_downloading: false,
    error: null,
  };
  const save = options.save ?? vi.fn().mockResolvedValue(true);
  const complete = vi.fn();
  let view!: ReturnType<typeof render>;
  await act(async () => {
    view = render(<Harness initial={initial} status={status} save={save} complete={complete} />);
  });
  return {
    ...view,
    initial,
    status,
    save,
    complete,
    statusChange: (changes: Partial<ModelStatus>) =>
      view.rerender(
        <Harness
          initial={initial}
          status={{ ...status, ...changes }}
          save={save}
          complete={complete}
        />,
      ),
  };
}
const next = async () => {
  await waitFor(() => expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled());
  await act(async () => fireEvent.click(screen.getByRole("button", { name: "Continue" })));
};
async function hardware() {
  fireEvent.click(screen.getByRole("button", { name: "Test microphone" }));
  await screen.findByText("Healthy");
  await next();
  fireEvent.click(screen.getByRole("button", { name: "Use the Record button instead" }));
  await next();
  await screen.findByText("Processor");
}
async function weights() {
  await hardware();
  await next();
  await screen.findByText(/Dependencies ready/);
  await waitFor(() => expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled());
  await next();
  await screen.findByRole("heading", { name: "Download your speech model" });
}

it("requires a usable microphone and supports retesting before continuing", async () => {
  vi.mocked(api.testMicrophone).mockResolvedValueOnce({
    ...healthy,
    rms: 0.001,
    assessment: "Signal is quiet",
  });
  await show();
  expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Test microphone" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Signal is quiet");
  expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Test microphone" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled());
  await next();
  expect(screen.getByRole("heading", { name: "Your dictation shortcut" })).toBeInTheDocument();
});

it("tests native shortcut liveness on a fresh install without ASR or recording", async () => {
  const start = vi.spyOn(api, "startRecording");
  await show();
  fireEvent.click(screen.getByRole("button", { name: "Test microphone" }));
  await screen.findByText("Healthy");
  await next();
  expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Test shortcut" }));
  await waitFor(() => expect(api.beginShortcutCapture).toHaveBeenCalledOnce());
  emit("hotkey:capture", { id: 71, action: "dictate", pressed: true });
  expect(screen.getByText(/Shortcut works/)).toBeInTheDocument();
  expect(start).not.toHaveBeenCalled();
  await next();
  expect(screen.getByRole("heading", { name: "Check your computer" })).toBeInTheDocument();
});

it("distinguishes deferral from verified completion and resumes through dependency verification", async () => {
  const view = await show();
  await hardware();
  await next();
  await screen.findByText(/Dependencies ready/);
  await waitFor(() => expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled());
  await next();
  fireEvent.click(screen.getByRole("button", { name: "Set up later" }));
  expect(view.complete).toHaveBeenCalledWith("deferred");
  expect(readSetupCheckpoint()).toMatchObject({
    stage: 4,
    outcome: "deferred",
    shortcut: { mode: "button" },
  });
  view.unmount();
  await show();
  await screen.findByRole("heading", { name: "Prepare speech dependencies" });
  await waitFor(() => expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled());
});

it("explains a blocked speech profile and clears the blocker after a compatible selection", async () => {
  const setup = vi.spyOn(api, "setupPythonRuntime").mockResolvedValue();
  plan.blocker = "This speech profile is unavailable here. Select Zipformer 20M.";
  const save = vi.fn(async (changes: Partial<AppSettings>) => {
    if (changes.asr?.model === "zipformer-20m") plan.blocker = null;
    return true;
  });
  await show({ save });
  await hardware();
  expect(await screen.findByRole("alert")).toHaveTextContent("Select Zipformer 20M");
  expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: /Zipformer 20M INT8/ }));
  await waitFor(() => expect(screen.queryByRole("alert")).not.toBeInTheDocument());
  await waitFor(() => expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled());
  expect(setup).not.toHaveBeenCalled();
});

it.each([
  { id: "phonon-2", label: "Phonon-2", size: "164 MB" },
  { id: "zipformer-20m", label: "Zipformer 20M INT8", size: "44 MB" },
])(
  "persists $label and its Python runtime before fetching its weights",
  async ({ id, label, size }) => {
    const base = await api.getSettings();
    const view = await show({
      initial: {
        ...base,
        offline_mode: false,
        asr: { ...base.asr, runtime: "native", precision: "int4" },
      },
    });
    await hardware();
    const card = screen.getByRole("button", { name: new RegExp(label) });
    expect(card).toHaveTextContent(size);
    expect(card).toHaveTextContent("English only");
    fireEvent.click(card);
    await waitFor(() =>
      expect(view.save).toHaveBeenCalledWith({
        asr: { ...base.asr, model: id, runtime: "python", precision: "auto" },
      }),
    );
    await waitFor(() => expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled());
    await next();
    await screen.findByText(/Dependencies ready/);
    await next();
    fireEvent.click(screen.getByRole("button", { name: "Download and set up" }));
    await waitFor(() => expect(api.installModel).toHaveBeenCalledExactlyOnceWith(id));
    view.statusChange({ is_downloading: true, download_progress_pct: 35 });
    expect(screen.getByText("Downloading 35%")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
  },
);

it("installs dependencies as a separate stage before exposing a model download", async () => {
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({ ...runtime, managed_installed: false });
  const setup = vi.spyOn(api, "setupPythonRuntime").mockResolvedValue();
  await show();
  await hardware();
  await next();
  fireEvent.click(await screen.findByRole("button", { name: "Download all dependencies" }));
  await waitFor(() => expect(setup).toHaveBeenCalledOnce());
  expect(api.installModel).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
  emit("python-runtime:setup-progress", {
    phase: "installing_packages",
  } satisfies PythonSetupProgress);
  expect(screen.getByText(/Installing speech packages/)).toBeInTheDocument();
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue(runtime);
  emit("python-runtime:setup-progress", { phase: "complete" } satisfies PythonSetupProgress);
  await waitFor(() => expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled());
  await next();
  fireEvent.click(screen.getByRole("button", { name: "Download and set up" }));
  await waitFor(() => expect(api.installModel).toHaveBeenCalledOnce());
  expect(setup).toHaveBeenCalledOnce();
});

it("reports a dependency failure and permits retry without fetching model weights", async () => {
  vi.mocked(api.getPythonRuntimeStatus).mockResolvedValue({ ...runtime, managed_installed: false });
  const setup = vi.spyOn(api, "setupPythonRuntime").mockResolvedValue();
  await show();
  await hardware();
  await next();
  fireEvent.click(await screen.findByRole("button", { name: "Download all dependencies" }));
  await waitFor(() => expect(setup).toHaveBeenCalledOnce());
  emit("python-runtime:setup-progress", { phase: "error", error: "Connection timed out" });
  expect(await screen.findByRole("alert")).toHaveTextContent("Connection timed out");
  expect(api.installModel).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Retry dependency setup" }));
  await waitFor(() => expect(setup).toHaveBeenCalledTimes(2));
});

it("offers a one-time download choice in airplane mode and respects a failed settings write", async () => {
  const initial = { ...(await api.getSettings()), offline_mode: true };
  const save = vi.fn().mockResolvedValue(false);
  await show({ initial, save });
  await weights();
  fireEvent.click(screen.getByRole("button", { name: "Allow downloads and set up" }));
  await waitFor(() => expect(save).toHaveBeenCalledWith({ offline_mode: false }));
  expect(api.installModel).not.toHaveBeenCalled();
  expect(screen.getByRole("alert")).toHaveTextContent(/could not be saved/);
});

it("surfaces background download failure and can retry or cancel the selected download", async () => {
  const cancel = vi.spyOn(api, "cancelModelDownload").mockResolvedValue();
  const view = await show();
  await weights();
  fireEvent.click(screen.getByRole("button", { name: "Download and set up" }));
  await waitFor(() => expect(api.installModel).toHaveBeenCalledOnce());
  view.statusChange({ is_downloading: true, download_progress_pct: 1 });
  fireEvent.click(screen.getByRole("button", { name: "Cancel model download" }));
  expect(cancel).toHaveBeenCalledWith(view.initial.asr.model);
  view.statusChange({ error: "Downloads are restricted to pinned artifact hosts" });
  expect(screen.getByRole("alert")).toHaveTextContent("pinned artifact hosts");
  fireEvent.click(screen.getByRole("button", { name: "Download and set up" }));
  await waitFor(() => expect(api.installModel).toHaveBeenCalledTimes(2));
});

it("requires a real dictation and explicit delivery choice to finish speech-only setup", async () => {
  const copy = vi.spyOn(api, "copyText").mockResolvedValue();
  const view = await show({ ready: true });
  await weights();
  await next();
  expect(screen.getByRole("button", { name: "Finish" })).toBeDisabled();
  fireEvent.change(screen.getByRole("textbox", { name: "Try dictation here" }), {
    target: { value: "Typed manually" },
  });
  expect(screen.getByRole("button", { name: "Finish" })).toBeDisabled();
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Record a sentence" })).toBeEnabled(),
  );
  emit("app:state-changed", "RECORDING");
  emit("injection:result", {
    session_id: 11,
    pasted: false,
    fallback_copy: true,
    message: "Copy required",
  });
  emit("transcript:source", { session_id: 11, dictation: true, final_text: "Hello world" });
  expect(screen.getByRole("button", { name: "Finish" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Back" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Set up later" })).toBeDisabled();
  emit("app:state-changed", "READY");
  fireEvent.click(screen.getByRole("button", { name: "Copy transcript and use clipboard mode" }));
  await waitFor(() => expect(screen.getByRole("button", { name: "Finish" })).toBeEnabled());
  expect(copy).toHaveBeenCalledWith("Hello world");
  expect(view.save).toHaveBeenCalledWith({ output_action: { type: "copy" } });
  fireEvent.click(screen.getByRole("button", { name: "Finish" }));
  expect(view.complete).toHaveBeenCalledWith("completed");
  expect(readSetupCheckpoint()).toMatchObject({ outcome: "completed", delivery: "clipboard" });
});

it("does not advance while microphone settings are being saved and invalidates its prior test", async () => {
  let resolve!: (ok: boolean) => void;
  const save = vi.fn(
    () =>
      new Promise<boolean>((yes) => {
        resolve = yes;
      }),
  );
  await show({ save });
  fireEvent.click(screen.getByRole("button", { name: "Test microphone" }));
  await screen.findByText("Healthy");
  fireEvent.change(screen.getByRole("combobox", { name: "Microphone" }), {
    target: { value: "usb" },
  });
  expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
  await act(async () => resolve(true));
  expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
});

it.each([
  { model: "0.6b", label: "Qwen3-ASR 0.6B" },
  { model: "zipformer-20m", label: "Zipformer 20M" },
])("shows and applies a compatible $model recommendation", async ({ model, label }) => {
  vi.mocked(api.recommendedAsrModel).mockResolvedValue(model);
  const base = await api.getSettings();
  const initial = { ...base, language: "en", asr: { ...base.asr, model: "1.7b" } };
  vi.spyOn(api, "getSettings").mockResolvedValue(initial);
  const view = await show({ initial });
  await waitFor(() =>
    expect(view.save).toHaveBeenCalledWith(
      expect.objectContaining({ asr: expect.objectContaining({ model }) }),
    ),
  );
  await hardware();
  expect(screen.getByRole("button", { name: new RegExp(label) })).toHaveTextContent("Recommended");
});

it("does not silently replace a non-English choice with Zipformer", async () => {
  vi.mocked(api.recommendedAsrModel).mockResolvedValue("zipformer-20m");
  const base = await api.getSettings();
  const initial = { ...base, language: "hi", asr: { ...base.asr, model: "1.7b" } };
  vi.spyOn(api, "getSettings").mockResolvedValue(initial);
  const view = await show({ initial });
  await hardware();
  expect(view.save).not.toHaveBeenCalled();
});

it("rechecks saved runtime progress and sends unavailable microphones back to input selection", async () => {
  const initial = { ...(await api.getSettings()), microphone_device_id: "removed" };
  writeSetupCheckpoint({
    version: 1,
    outcome: "deferred",
    stage: 4,
    microphone: microphoneConfiguration(initial),
    shortcut: { configuration: shortcutConfiguration(initial), mode: "button" },
    speech: speechConfiguration(initial),
  });
  await show({ initial });
  expect(await screen.findByRole("alert")).toHaveTextContent("microphone is unavailable");
  expect(screen.getByRole("heading", { name: "Choose a microphone" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
});
