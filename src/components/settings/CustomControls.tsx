import React from "react";
import { ComputeDevice, Precision, ProfileOverrides } from "../../types";
import { Button } from "../ui/Button";
import { Field } from "../ui/Field";
import { cn } from "../../utils/cn";
import { RotateCcw, Sliders } from "lucide-react";

interface CustomControlsProps {
  overrides: ProfileOverrides;
  onChangeOverrides: (overrides: ProfileOverrides) => void;
  className?: string;
}

export function CustomControls({ overrides, onChangeOverrides, className }: CustomControlsProps) {
  const updateField = <K extends keyof ProfileOverrides>(key: K, value: ProfileOverrides[K]) => {
    onChangeOverrides({
      ...overrides,
      [key]: value,
    });
  };

  const handleReset = () => {
    onChangeOverrides({});
  };

  const gpuLayers = overrides.refinement_gpu_layers ?? 0;

  return (
    <div className={cn("space-y-4", className)}>
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Sliders className="w-5 h-5 text-accent" />
          <h3 className="font-semibold text-ink">Manual Runtime Overrides</h3>
        </div>
        <Button size="sm" variant="ghost" onClick={handleReset} className="text-xs">
          <RotateCcw className="w-3.5 h-3.5 mr-1" /> Reset to Defaults
        </Button>
      </div>

      <div className="p-4 rounded-xl bg-surface border border-line space-y-4">
        {/* ASR Compute Device & Precision */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          <Field label="ASR Compute Device" hint="Hardware backend targeting speech recognition">
            <select
              value={overrides.asr_device ?? "auto"}
              onChange={(e) =>
                updateField(
                  "asr_device",
                  e.target.value === "auto" ? null : (e.target.value as ComputeDevice),
                )
              }
              className="w-full px-3 py-2 text-sm bg-surface-2 border border-line rounded-lg text-ink focus:outline-none focus:ring-2 focus:ring-accent"
            >
              <option value="auto">Auto (Adaptive recommendation)</option>
              <option value="cuda">NVIDIA CUDA (GPU)</option>
              <option value="cpu">CPU (AVX2 / Multi-thread)</option>
              <option value="vulkan">Vulkan Compute</option>
            </select>
          </Field>

          <Field label="Model Precision" hint="Floating-point quantization format">
            <select
              value={overrides.asr_precision ?? "auto"}
              onChange={(e) =>
                updateField(
                  "asr_precision",
                  e.target.value === "auto" ? null : (e.target.value as Precision),
                )
              }
              className="w-full px-3 py-2 text-sm bg-surface-2 border border-line rounded-lg text-ink focus:outline-none focus:ring-2 focus:ring-accent"
            >
              <option value="auto">Auto (Hardware native)</option>
              <option value="bf16">BF16 (Bfloat16 - Recommended)</option>
              <option value="fp16">FP16 (Half precision)</option>
              <option value="fp32">FP32 (Full single precision)</option>
            </select>
          </Field>
        </div>

        {/* Refinement Device & GPU Offload */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4 pt-2 border-t border-line-soft">
          <Field
            label="Refinement Compute Device"
            hint="Hardware backend for AI polish / punctuation"
          >
            <select
              value={overrides.refinement_device ?? "auto"}
              onChange={(e) =>
                updateField(
                  "refinement_device",
                  e.target.value === "auto" ? null : (e.target.value as ComputeDevice),
                )
              }
              className="w-full px-3 py-2 text-sm bg-surface-2 border border-line rounded-lg text-ink focus:outline-none focus:ring-2 focus:ring-accent"
            >
              <option value="auto">Auto (Adaptive)</option>
              <option value="cuda">NVIDIA CUDA</option>
              <option value="cpu">CPU</option>
              <option value="vulkan">Vulkan</option>
            </select>
          </Field>

          <Field
            label={`GPU Layer Offload (${gpuLayers} / 32 layers)`}
            hint="Number of transformer layers offloaded to GPU VRAM"
          >
            <div className="flex items-center gap-3 pt-1">
              <input
                type="range"
                min={0}
                max={32}
                step={1}
                value={gpuLayers}
                onChange={(e) => updateField("refinement_gpu_layers", parseInt(e.target.value, 10))}
                className="w-full accent-sky-500 cursor-pointer"
              />
              <span className="text-xs font-mono font-medium text-ink-2 w-8 text-right">
                {gpuLayers}
              </span>
            </div>
          </Field>
        </div>

        {/* Streaming Behavior */}
        <div className="pt-2 border-t border-line-soft">
          <Field
            label="Live Partial Streaming"
            hint="Show immediate transcription tokens while holding the hotkey"
          >
            <select
              value={
                overrides.force_streaming === undefined || overrides.force_streaming === null
                  ? "auto"
                  : overrides.force_streaming
                    ? "true"
                    : "false"
              }
              onChange={(e) =>
                updateField(
                  "force_streaming",
                  e.target.value === "auto" ? null : e.target.value === "true",
                )
              }
              className="w-full px-3 py-2 text-sm bg-surface-2 border border-line rounded-lg text-ink focus:outline-none focus:ring-2 focus:ring-accent"
            >
              <option value="auto">Auto (Enabled when hardware latency &lt; 250ms)</option>
              <option value="true">Force Always Enabled</option>
              <option value="false">Disabled (Batch finalize only)</option>
            </select>
          </Field>
        </div>
      </div>
    </div>
  );
}
