import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { SetupProgress, writingSetupLabel } from "./SetupProgress";
import type { IntelligenceDownloadEvent } from "../types";
const event = (phase: IntelligenceDownloadEvent["phase"]): IntelligenceDownloadEvent => ({
  tier: "qwen3.5-4b",
  phase,
  progress_pct: 0,
  speed_mbps: 0,
});
it("shows the runtime prerequisite before any model progress", () => {
  render(<SetupProgress writing={event("installing_runtime")} onReview={vi.fn()} />);
  expect(screen.getByRole("status")).toHaveTextContent("Writing runtime");
  expect(screen.getByText(/Model download follows runtime verification/)).toBeVisible();
  expect(screen.queryByText("Downloading model")).not.toBeInTheDocument();
});
it("shows model progress only during the model phase", () => {
  render(
    <SetupProgress writing={{ ...event("downloading"), progress_pct: 42 }} onReview={vi.fn()} />,
  );
  expect(screen.getByText("Downloading model")).toBeVisible();
  expect(screen.getByText("42%")).toBeVisible();
});
it("keeps terminal failures actionable without restarting automatically", () => {
  const review = vi.fn();
  render(
    <SetupProgress
      writing={{ ...event("error"), error: "Runtime checksum failed" }}
      onReview={review}
    />,
  );
  expect(screen.getByRole("alert")).toHaveTextContent("Runtime checksum failed");
  expect(review).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Review and retry" }));
  expect(review).toHaveBeenCalledOnce();
});
it("does not interpret an unknown phase as a model download", () => {
  expect(writingSetupLabel("future_phase")).toBe("Checking setup status");
});
it("removes completed work from the sidebar", () => {
  render(<SetupProgress writing={event("complete")} onReview={vi.fn()} />);
  expect(screen.queryByRole("status")).not.toBeInTheDocument();
});

it("shows the active runtime retry instead of a prior writing failure", () => {
  render(
    <SetupProgress
      writing={event("error")}
      runtime={
        { phase: "downloading", progress_pct: 30 } as import("../types").RuntimeDownloadEvent
      }
      onReview={() => {}}
    />,
  );
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("Installing and verifying runtime");
});
