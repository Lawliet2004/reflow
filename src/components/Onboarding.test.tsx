import { act, fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { api } from "../services/tauriApi";
import { Onboarding } from "./Onboarding";

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
