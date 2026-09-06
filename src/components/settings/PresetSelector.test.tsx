import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { describe, expect, it, vi } from "vitest";
import { PresetSelector } from "./PresetSelector";

vi.mock("../../services/tauriApi", () => ({
  tauriApi: {
    previewProfile: vi.fn().mockResolvedValue({
      preset: "auto",
      asr_model: "qwen3-asr-0.6b",
      asr_device: "cuda",
      asr_precision: "bf16",
      asr_attempts: [],
      refinement_model: "qwen3.5-0.8b",
      refinement_device: "cpu",
      refinement_gpu_layers: 0,
      language: "en",
      streaming_enabled: true,
      inference_threads: 4,
      reasons: [],
    }),
  },
}));

describe("PresetSelector", () => {
  it("renders all preset cards and handles selection", () => {
    const onSelect = vi.fn();
    render(<PresetSelector selectedPreset="auto" onSelectPreset={onSelect} />);

    expect(screen.getByText("Auto (Adaptive)")).toBeInTheDocument();
    expect(screen.getByText("Fast")).toBeInTheDocument();
    expect(screen.getByText("Balanced")).toBeInTheDocument();
    expect(screen.getByText("Accurate")).toBeInTheDocument();
    expect(screen.getByText("Custom")).toBeInTheDocument();

    const fastOption = screen.getByText("Fast");
    fireEvent.click(fastOption);
    expect(onSelect).toHaveBeenCalledWith("fast");
  });
});
