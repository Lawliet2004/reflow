import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { api } from "../services/tauriApi";
import { Onboarding } from "./Onboarding";

// Keep the recommendation probe from preselecting 0.6B underneath tests that
// exercise the download flow — those tests pick their model explicitly.
beforeEach(() => {
  vi.spyOn(api, "recommendedAsrModel").mockResolvedValue("1.7b");
});

it.each([
  { id: "phonon-2", name: "Phonon-2", size: "164 MB" },
  { id: "zipformer-20m", name: "Zipformer 20M INT8", size: "44 MB" },
])("downloads $name during first-time setup after saving its runtime settings", async (model) => {
  const settings = await api.getSettings();
  const status = await api.getModelStatus();
  const initial = {
    ...settings,
    offline_mode: false,
    asr: { ...settings.asr, runtime: "native" as const, precision: "int4" as const },
  };
  const missing = {
    ...status,
    installed: false,
    loaded: false,
    is_loading: false,
    is_downloading: false,
    error: null,
  };
  const save = vi.fn().mockResolvedValue(true);
  const install = vi.spyOn(api, "installModel").mockResolvedValue();
  const props = { onUpdateSettings: save, onComplete: vi.fn() };
  const view = render(<Onboarding {...props} settings={initial} modelStatus={missing} />);
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  expect(screen.getByRole("button", { name: /Qwen3-ASR 0.6B/ })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /Qwen3-ASR 1.7B/ })).toBeInTheDocument();
  const option = screen.getByRole("button", { name: new RegExp(model.name) });
  expect(option).toHaveTextContent("English only");
  expect(option).toHaveTextContent("CPU");
  expect(option).toHaveTextContent(model.size);
  fireEvent.click(option);
  await waitFor(() => expect(screen.queryByText("Saving settings…")).not.toBeInTheDocument());
  expect(save).toHaveBeenCalledExactlyOnceWith({
    asr: { ...initial.asr, model: model.id, runtime: "python", precision: "auto" },
  });
  view.rerender(
    <Onboarding
      {...props}
      settings={{ ...initial, ...save.mock.calls[0][0] }}
      modelStatus={missing}
    />,
  );
  expect(screen.getByRole("button", { name: new RegExp(model.name) })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  fireEvent.click(screen.getByRole("button", { name: "Download model" }));
  await waitFor(() => expect(install).toHaveBeenCalledExactlyOnceWith(model.id));
  view.rerender(
    <Onboarding
      {...props}
      settings={{ ...initial, ...save.mock.calls[0][0] }}
      modelStatus={{ ...missing, is_downloading: true, download_progress_pct: 35 }}
    />,
  );
  expect(screen.getByText("Downloading 35%")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /Qwen3-ASR 0.6B/ })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Finish" })).toBeDisabled();
});

it("shows background download failures and permits retrying the selected speech model", async () => {
  const settings = await api.getSettings();
  const status = await api.getModelStatus();
  const install = vi.spyOn(api, "installModel").mockResolvedValue();
  const props = {
    settings: {
      ...settings,
      offline_mode: false,
      asr: { ...settings.asr, model: "1.7b" as const },
    },
    onUpdateSettings: vi.fn(),
    onComplete: vi.fn(),
  };
  const missing = {
    ...status,
    installed: false,
    loaded: false,
    is_downloading: false,
    error: null,
  };
  const view = render(<Onboarding {...props} modelStatus={missing} />);
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Download model" }));
  await waitFor(() => expect(install).toHaveBeenCalledExactlyOnceWith("1.7b"));
  view.rerender(
    <Onboarding
      {...props}
      modelStatus={{ ...missing, is_downloading: true, download_progress_pct: 1 }}
    />,
  );
  expect(screen.getByText("Downloading 1%")).toBeInTheDocument();
  view.rerender(
    <Onboarding
      {...props}
      modelStatus={{ ...missing, error: "Downloads are restricted to pinned artifact hosts" }}
    />,
  );
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Downloads are restricted to pinned artifact hosts",
  );
  fireEvent.click(screen.getByRole("button", { name: "Download model" }));
  await waitFor(() => expect(install).toHaveBeenCalledTimes(2));
});

it("asks for one-time download consent when airplane mode is on", async () => {
  const settings = await api.getSettings();
  expect(settings.offline_mode).toBe(true);
  const status = await api.getModelStatus();
  const save = vi.fn().mockResolvedValue(true);
  const install = vi.spyOn(api, "installModel").mockResolvedValue();
  render(
    <Onboarding
      settings={settings}
      modelStatus={{ ...status, installed: false, loaded: false }}
      onUpdateSettings={save}
      onComplete={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  expect(screen.queryByRole("button", { name: "Download model" })).not.toBeInTheDocument();
  const allow = screen.getByRole("button", { name: "Allow downloads" });
  fireEvent.click(allow);
  await waitFor(() => expect(save).toHaveBeenCalledExactlyOnceWith({ offline_mode: false }));
  await waitFor(() => expect(install).toHaveBeenCalledExactlyOnceWith(settings.asr.model));
});

it("shows the consent card instead of a raw error when a download is rejected as offline", async () => {
  const settings = { ...(await api.getSettings()), offline_mode: false };
  const status = await api.getModelStatus();
  vi.spyOn(api, "installModel").mockRejectedValue("Offline mode is on");
  render(
    <Onboarding
      settings={settings}
      modelStatus={{ ...status, installed: false, loaded: false }}
      onUpdateSettings={vi.fn()}
      onComplete={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Download model" }));
  expect(await screen.findByRole("button", { name: "Allow downloads" })).toBeInTheDocument();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

it("offers 'Set up later' when the model step cannot be completed", async () => {
  const settings = { ...(await api.getSettings()), offline_mode: false };
  const status = await api.getModelStatus();
  const complete = vi.fn();
  render(
    <Onboarding
      settings={settings}
      modelStatus={{ ...status, installed: false, loaded: false }}
      onUpdateSettings={vi.fn()}
      onComplete={complete}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  expect(screen.getByRole("button", { name: "Finish" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Set up later" }));
  expect(complete).toHaveBeenCalledOnce();
});

it("finishes speech-only setup once recognition passes, without the retired LLM picker", async () => {
  const settings = await api.getSettings();
  const status = await api.getModelStatus();
  const save = vi.fn().mockResolvedValue(true);
  const complete = vi.fn();
  vi.spyOn(api, "testRecognition").mockResolvedValue({
    text: "Ready to dictate",
    language: "en",
    health: {
      device: "Mic",
      duration_ms: 3000,
      peak: 0.5,
      rms: 0.1,
      clipped_pct: 0,
      dropped_chunks: 0,
      assessment: "Healthy",
    },
  });
  render(
    <Onboarding
      settings={settings}
      modelStatus={{ ...status, installed: true, loaded: true, is_loading: false }}
      onUpdateSettings={save}
      onComplete={complete}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  expect(screen.queryByRole("combobox", { name: "LLM model" })).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Test recognition" }));
  await screen.findByText("Ready to dictate");
  await waitFor(() => expect(screen.getByRole("button", { name: "Finish" })).toBeEnabled());
  fireEvent.click(screen.getByRole("button", { name: "Finish" }));
  expect(complete).toHaveBeenCalledOnce();
});

it("requires usable recognition rather than only installed weights before finishing", async () => {
  const settings = await api.getSettings();
  const status = await api.getModelStatus();
  const complete = vi.fn();
  const test = vi
    .spyOn(api, "testRecognition")
    .mockRejectedValueOnce(new Error("No speech"))
    .mockResolvedValueOnce({
      text: "Hello world",
      language: "en",
      health: {
        device: "Mic",
        duration_ms: 3000,
        peak: 0.5,
        rms: 0.1,
        clipped_pct: 0,
        dropped_chunks: 0,
        assessment: "Healthy",
      },
    });
  const props = { settings, onUpdateSettings: vi.fn(), onComplete: complete };
  const view = render(
    <Onboarding {...props} modelStatus={{ ...status, installed: true, loaded: false }} />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  expect(screen.getByRole("button", { name: "Finish" })).toBeDisabled();
  view.rerender(
    <Onboarding
      {...props}
      modelStatus={{ ...status, installed: true, loaded: true, is_loading: false }}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Test recognition" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("No speech");
  expect(screen.getByRole("button", { name: "Finish" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Test recognition" }));
  expect(await screen.findByText("Hello world")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Finish" }));
  expect(complete).toHaveBeenCalledOnce();
  expect(test).toHaveBeenCalledTimes(2);
});

it("cannot reuse a successful recognition check while a new model selection is being saved", async () => {
  const settings = await api.getSettings();
  const status = await api.getModelStatus();
  const complete = vi.fn();
  let resolve!: (saved: boolean) => void;
  const save = vi.fn(
    () =>
      new Promise<boolean>((yes) => {
        resolve = yes;
      }),
  );
  const recognition = vi.spyOn(api, "testRecognition").mockResolvedValue({
    text: "Working microphone",
    language: "en",
    health: {
      device: "Mic",
      duration_ms: 3000,
      peak: 0.5,
      rms: 0.1,
      clipped_pct: 0,
      dropped_chunks: 0,
      assessment: "Healthy",
    },
  });
  render(
    <Onboarding
      settings={settings}
      modelStatus={{ ...status, installed: true, loaded: true, is_loading: false }}
      onUpdateSettings={save}
      onComplete={complete}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Test recognition" }));
  expect(await screen.findByText("Working microphone")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Finish" })).toBeEnabled();

  // The parent may be waiting for queued disk persistence and still expose its old settings.
  fireEvent.click(screen.getByRole("button", { name: /1.7B/ }));
  expect(save).toHaveBeenCalledOnce();
  expect(screen.getByRole("button", { name: "Finish" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Test recognition" })).toBeDisabled();
  fireEvent.click(screen.getByRole("button", { name: "Finish" }));
  fireEvent.click(screen.getByRole("button", { name: "Test recognition" }));
  expect(complete).not.toHaveBeenCalled();
  expect(recognition).toHaveBeenCalledOnce();
  await act(async () => {
    resolve(false);
  });
  expect(await screen.findByRole("alert")).toHaveTextContent(/save|saved/i);
  // A failed selection must require a new recognition test, even if the old props remain.
  expect(screen.getByRole("button", { name: "Finish" })).toBeDisabled();
});

it("blocks advancing from microphone selection while its settings write is pending", async () => {
  const settings = await api.getSettings();
  const status = await api.getModelStatus();
  let resolve!: (saved: boolean) => void;
  const save = vi.fn(
    () =>
      new Promise<boolean>((yes) => {
        resolve = yes;
      }),
  );
  vi.spyOn(api, "getAudioDevices").mockResolvedValue([
    { id: "usb", name: "USB microphone", is_default: true, sample_rate: 48000, channels: 1 },
  ]);
  render(
    <Onboarding
      settings={settings}
      modelStatus={status}
      onUpdateSettings={save}
      onComplete={vi.fn()}
    />,
  );
  await screen.findByRole("option", { name: /USB microphone/ });
  fireEvent.change(screen.getByRole("combobox", { name: "Microphone" }), {
    target: { value: "usb" },
  });
  expect(screen.getByRole("combobox", { name: "Microphone" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
  await act(async () => {
    resolve(true);
  });
  expect(screen.getByRole("button", { name: "Continue" })).toBeEnabled();
});

it("preselects the recommended model when hardware does not fit the default", async () => {
  vi.mocked(api.recommendedAsrModel).mockResolvedValue("0.6b");
  const settings = await api.getSettings();
  const status = await api.getModelStatus();
  // The preselect re-reads the backend rather than the mount-time snapshot —
  // keep the mock's stored selection on the factory default for this test.
  vi.spyOn(api, "getSettings").mockResolvedValue({
    ...settings,
    asr: { ...settings.asr, model: "1.7b" as const },
  });
  const save = vi.fn().mockResolvedValue(true);
  render(
    <Onboarding
      settings={{ ...settings, asr: { ...settings.asr, model: "1.7b" as const } }}
      modelStatus={{ ...status, installed: false, loaded: false }}
      onUpdateSettings={save}
      onComplete={vi.fn()}
    />,
  );
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({ asr: expect.objectContaining({ model: "0.6b" }) }),
    ),
  );
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  expect(screen.getByRole("button", { name: /Qwen3-ASR 0.6B/ })).toHaveTextContent("Recommended");
});

it("confirms a shortcut press during the hotkey step and cancels the probe session", async () => {
  const listeners = new Map<string, (payload: unknown) => void>();
  const bridge = await import("../services/tauriApi");
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    listeners.set(event as string, handler as (payload: unknown) => void);
    return () => {};
  });
  const cancel = vi.spyOn(api, "cancelRecording").mockResolvedValue();
  const settings = await api.getSettings();
  const status = await api.getModelStatus();
  render(
    <Onboarding
      settings={settings}
      modelStatus={status}
      onUpdateSettings={vi.fn()}
      onComplete={vi.fn()}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Continue" }));
  expect(screen.getByText(/Press your shortcut anywhere/)).toBeInTheDocument();
  await act(async () => listeners.get("app:state-changed")?.("RECORDING"));
  expect(screen.getByText(/Shortcut detected/)).toBeInTheDocument();
  await waitFor(() => expect(cancel).toHaveBeenCalled());
});
