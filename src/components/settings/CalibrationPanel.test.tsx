import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { api } from "../../services/tauriApi";
import { CalibrationPanel } from "./CalibrationPanel";

const idle = {
  running: false,
  phase: "idle",
  candidates: [],
  winner_id: null,
  error: null,
  language: "en",
  reference: "",
};
const winner = {
  id: "asr-bf16-llm-cpu",
  label: "ASR BF16 + LLM CPU",
  median_ms: 120,
  p95_ms: 140,
  wer: 0,
  cer: 0,
  eligible: true,
  error: null,
};
const complete = { ...idle, phase: "complete", candidates: [winner], winner_id: winner.id };

it("requires a reference before explicitly recording and applying a qualified result", async () => {
  vi.spyOn(api, "getCalibrationStatus").mockResolvedValue(idle);
  let finish!: (value: typeof complete) => void;
  const run = vi.spyOn(api, "runCalibration").mockReturnValue(
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  const apply = vi.spyOn(api, "applyCalibration").mockResolvedValue(true);
  render(<CalibrationPanel />);
  const reference = await screen.findByRole("textbox", { name: "Reference transcript" });
  const start = screen.getByRole("button", { name: "Run calibration" });
  expect(start).toBeDisabled();
  expect(run).not.toHaveBeenCalled();
  fireEvent.change(reference, { target: { value: "We will ship the update on Thursday." } });
  fireEvent.change(screen.getByRole("combobox", { name: "Calibration language" }), {
    target: { value: "en" },
  });
  fireEvent.click(start);
  expect(await screen.findByRole("button", { name: "Cancel calibration" })).toBeEnabled();
  expect(
    screen.queryByRole("button", { name: "Apply fastest qualified settings" }),
  ).not.toBeInTheDocument();
  finish(complete);
  const applyButton = await screen.findByRole("button", {
    name: "Apply fastest qualified settings",
  });
  expect(screen.getByText(winner.label)).toBeInTheDocument();
  fireEvent.click(applyButton);
  await waitFor(() => expect(apply).toHaveBeenCalledWith(winner.id));
  expect(run).toHaveBeenCalledWith(
    expect.objectContaining({ reference: "We will ship the update on Thursday.", language: "en" }),
  );
});

it("offers no Apply action when every faster candidate fails the quality gate", async () => {
  vi.spyOn(api, "getCalibrationStatus").mockResolvedValue({
    ...complete,
    winner_id: null,
    candidates: [
      { ...winner, eligible: false, error: "Transcript accuracy did not meet the quality gate" },
    ],
  });
  render(<CalibrationPanel />);
  expect(
    await screen.findByText("Transcript accuracy did not meet the quality gate"),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Apply fastest qualified settings" }),
  ).not.toBeInTheDocument();
});

it("preserves the reference and exposes a failed run for correction and retry", async () => {
  vi.spyOn(api, "getCalibrationStatus").mockResolvedValue(idle);
  vi.spyOn(api, "runCalibration").mockRejectedValue(new Error("Selected microphone disconnected"));
  render(<CalibrationPanel />);
  const reference = await screen.findByRole("textbox", { name: "Reference transcript" });
  fireEvent.change(reference, { target: { value: "This is my reference phrase." } });
  fireEvent.click(screen.getByRole("button", { name: "Run calibration" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Selected microphone disconnected");
  expect(reference).toHaveValue("This is my reference phrase.");
  expect(screen.getByRole("button", { name: "Run calibration" })).toBeEnabled();
});

it("settles a calibration started before the panel was reopened", async () => {
  vi.spyOn(api, "getCalibrationStatus")
    .mockResolvedValueOnce({ ...idle, running: true, phase: "Measuring", reference: "My phrase" })
    .mockResolvedValue(complete);
  render(<CalibrationPanel />);
  expect(await screen.findByRole("button", { name: "Cancel calibration" })).toBeEnabled();
  await waitFor(
    () => {
      expect(screen.getByRole("button", { name: "Run calibration" })).toBeEnabled();
      expect(screen.queryByRole("button", { name: "Cancel calibration" })).not.toBeInTheDocument();
    },
    { timeout: 2500 },
  );
  expect(screen.getByRole("button", { name: "Apply fastest qualified settings" })).toBeEnabled();
});
