import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { describe, expect, it, vi } from "vitest";
import { DiagnosticsWaterfall } from "./DiagnosticsWaterfall";

vi.mock("../../services/tauriApi", () => ({
  tauriApi: {
    getLatencyReport: vi.fn().mockResolvedValue({
      last: {
        hotkey_to_recording_ms: 10,
        recording_to_first_audio_ms: 20,
        audio_to_first_partial_ms: 50,
        speech_end_to_final_ms: 120,
        final_to_injection_ms: 15,
        llm_startup_ms: 0,
        formatting_ms: 5,
        rewrite_ms: 180,
        rewrite_applied: true,
        release_to_inserted_ms: 320,
        total_duration_ms: 1500,
        audio_duration_ms: 1200,
        rtf: 0.25,
        segments: [],
        last_updated: new Date().toISOString(),
      },
      percentiles: {
        samples: 10,
        hotkey_to_recording_p50_ms: 10,
        hotkey_to_recording_p95_ms: 15,
        speech_end_to_final_p50_ms: 110,
        speech_end_to_final_p95_ms: 140,
        rewrite_p50_ms: 170,
        rewrite_p95_ms: 210,
        release_to_inserted_p50_ms: 310,
        release_to_inserted_p95_ms: 360,
        rtf_p50: 0.24,
        rtf_p95: 0.28,
        llm_applied_rate: 0.9,
      },
      recent: [],
    }),
    getSystemMetrics: vi.fn().mockResolvedValue({
      cpu_usage_pct: 12,
      app_ram_mb: 150,
      model_ram_mb: 800,
      total_ram_mb: 16384,
      used_ram_mb: 6144,
      available_ram_mb: 10240,
      vram_mb: 2048,
      total_vram_mb: 8192,
      used_vram_mb: 2048,
      free_vram_mb: 6144,
      gpu_name: "NVIDIA RTX 4070",
      gpu_vendor: "nvidia",
      gpu_present: true,
      cuda_available: true,
      vulkan_available: true,
      cpu_model: "AMD Ryzen 7 7800X3D",
      physical_cores: 8,
      logical_cores: 16,
      asr_ram_mb: 500,
      refinement_ram_mb: 300,
      model_loaded: true,
      backend_name: "cuda",
      os_name: "windows",
      session: "active",
    }),
    resetLatencyHistory: vi.fn().mockResolvedValue({
      last: null,
      percentiles: null,
      recent: [],
    }),
  },
}));

describe("DiagnosticsWaterfall", () => {
  it("renders latency diagnostics and responds to reset", async () => {
    render(<DiagnosticsWaterfall />);

    expect(await screen.findByText("Live Latency & Diagnostics")).toBeInTheDocument();
    expect(await screen.findByText(/Last Dictation Timeline/)).toBeInTheDocument();

    const resetBtn = screen.getByText("Reset History");
    fireEvent.click(resetBtn);
  });
});
