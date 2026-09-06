import { render, screen } from "@testing-library/react";
import React from "react";
import { describe, expect, it, vi } from "vitest";
import { HardwareInspector } from "./HardwareInspector";

vi.mock("../../services/tauriApi", () => ({
  tauriApi: {
    getCapabilities: vi.fn().mockResolvedValue({
      gpus: [
        {
          index: 0,
          name: "NVIDIA GeForce RTX 4070",
          vendor: "nvidia",
          total_vram_mb: 12288,
          used_vram_mb: 3072,
          free_vram_mb: 9216,
          driver_version: "550.54",
          compute_capability: "8.9",
        },
      ],
      cuda: {
        nvidia_gpu_present: true,
        driver_present: true,
        driver_cuda_version: "12.4",
        torch_cuda_available: true,
        torch_cuda_version: "12.1",
      },
      vulkan: {
        available: true,
        api_version: "1.3",
        devices: ["NVIDIA GeForce RTX 4070"],
      },
      cpu: {
        model: "AMD Ryzen 7 7800X3D",
        physical_cores: 8,
        logical_cores: 16,
        load_pct: 8.5,
      },
      ram: {
        total_mb: 32768,
        used_mb: 12288,
        available_mb: 20480,
      },
      os_name: "Windows 11",
      probed_at: new Date().toISOString(),
    }),
    refreshCapabilities: vi.fn(),
  },
}));

describe("HardwareInspector", () => {
  it("renders hardware specifications and VRAM bar", async () => {
    render(<HardwareInspector />);

    expect(await screen.findByText("Hardware & Acceleration")).toBeInTheDocument();
    expect(await screen.findByText("NVIDIA GeForce RTX 4070")).toBeInTheDocument();
    expect(await screen.findByText("AMD Ryzen 7 7800X3D")).toBeInTheDocument();
    expect(await screen.findByText("CUDA Active")).toBeInTheDocument();
  });
});
