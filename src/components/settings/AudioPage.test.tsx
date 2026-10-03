import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { api } from "../../services/tauriApi";
import { AudioPage } from "./AudioPage";

async function audioPage() {
  const settings = await api.getSettings();
  const save = vi.fn();
  render(<AudioPage settings={settings} onUpdateSettings={save} />);
  return save;
}

it("offers all 30 supported speech languages and saves explicit selection", async () => {
  const save = await audioPage();
  const language = screen.getByRole("combobox", { name: "Dictation language" });
  expect(within(language).getAllByRole("option")).toHaveLength(31);
  for (const name of ["Arabic", "Cantonese", "Filipino", "Macedonian", "Persian", "Romanian"]) {
    expect(within(language).getByRole("option", { name })).toBeInTheDocument();
  }
  fireEvent.change(language, { target: { value: "ja" } });
  expect(save).toHaveBeenLastCalledWith({ language: "ja", auto_detect_language: false });
  fireEvent.change(language, { target: { value: "auto" } });
  expect(save).toHaveBeenLastCalledWith({ language: "auto", auto_detect_language: true });
});

it("shows microphone enumeration errors with a retry action", async () => {
  const devices = vi
    .spyOn(api, "getAudioDevices")
    .mockRejectedValueOnce(new Error("denied"))
    .mockResolvedValueOnce([
      { id: "usb", name: "USB microphone", is_default: true, sample_rate: 48000, channels: 1 },
    ]);
  await audioPage();
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not list microphones");
  fireEvent.click(screen.getByRole("button", { name: "Refresh microphones" }));
  await waitFor(() =>
    expect(screen.getByRole("option", { name: /USB microphone/ })).toBeInTheDocument(),
  );
  expect(devices).toHaveBeenCalledTimes(2);
});

it("reports measured microphone health and recovers after a failed test", async () => {
  const test = vi
    .spyOn(api, "testMicrophone")
    .mockRejectedValueOnce(new Error("Microphone unavailable"))
    .mockResolvedValueOnce({
      device: "USB microphone",
      duration_ms: 3000,
      peak: 0.7,
      rms: 0.1,
      clipped_pct: 0,
      dropped_chunks: 0,
      assessment: "Audio signal looks healthy.",
    });
  await audioPage();
  fireEvent.click(screen.getByRole("button", { name: "Test microphone" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Microphone unavailable");
  fireEvent.click(screen.getByRole("button", { name: "Test microphone" }));
  expect(await screen.findByText("Audio signal looks healthy.")).toBeInTheDocument();
  expect(screen.getByText(/Peak 70%/)).toBeInTheDocument();
  expect(test).toHaveBeenCalledTimes(2);
});
