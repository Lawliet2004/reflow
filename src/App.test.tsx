import { render, screen, fireEvent, waitFor, act } from "@testing-library/react";
import { it, expect, vi } from "vitest";
import { App } from "./App";
import { api } from "./services/tauriApi";

it("offers retry when startup settings fail and recovers", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "getSettings")
    .mockRejectedValueOnce(new Error("unavailable"))
    .mockResolvedValue(settings);
  render(<App />);
  await screen.findByRole("button", { name: "Try again" });
  fireEvent.click(screen.getByRole("button", { name: "Try again" }));
  await screen.findByRole("heading", { name: /Speak freely/ });
  expect(screen.queryByText("Starting Reflow…")).not.toBeInTheDocument();
});

it("keeps the shell usable when model status is unavailable", async () => {
  vi.spyOn(api, "getModelStatus").mockRejectedValue(new Error("sidecar unavailable"));
  render(<App />);
  await screen.findByRole("button", { name: "History" });
  fireEvent.click(screen.getByRole("button", { name: "History" }));
  await screen.findByRole("heading", { name: "A thought worth keeping." });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 200));
  });
  await waitFor(() => expect(screen.queryByText("Searching transcripts…")).not.toBeInTheDocument());
});

it("serializes preference changes and disables auto-detection for an explicit language", async () => {
  const initial = await api.getSettings();
  let releaseFirst!: (value: typeof initial) => void;
  const update = vi
    .spyOn(api, "updateSettings")
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          releaseFirst = resolve;
        }),
    )
    .mockResolvedValue({ ...initial, language: "hi", auto_detect_language: false });
  render(<App />);
  const language = await screen.findByRole("combobox", { name: "Language" });
  fireEvent.change(language, { target: { value: "en" } });
  await waitFor(() => expect(update).toHaveBeenCalledTimes(1));
  fireEvent.change(language, { target: { value: "hi" } });
  expect(update).toHaveBeenCalledTimes(1);
  await act(async () => {
    releaseFirst({ ...initial, language: "en", auto_detect_language: false });
  });
  await waitFor(() => expect(update).toHaveBeenCalledTimes(2));
  expect(update).toHaveBeenLastCalledWith({ language: "hi", auto_detect_language: false });
  expect(language).toHaveValue("hi");
});
