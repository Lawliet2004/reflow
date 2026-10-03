import React, { useEffect, useState } from "react";
import { tauriApi as api } from "../../services/tauriApi";
import { Capabilities, gpuPresentButCudaUnavailable, primaryGpu } from "../../types";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { cn } from "../../utils/cn";
import { AlertTriangle, Cpu, HardDrive, Monitor, RefreshCw } from "lucide-react";

export function HardwareInspector({ className }: { className?: string }) {
  const [caps, setCaps] = useState<Capabilities | null>(null);
  const [loading, setLoading] = useState(false);

  const fetchCapabilities = async (forceRefresh = false) => {
    setLoading(true);
    try {
      const data = forceRefresh ? await api.refreshCapabilities() : await api.getCapabilities();
      setCaps(data);
    } catch (e) {
      console.error("failed to get hardware capabilities", e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    let alive = true;
    queueMicrotask(() => {
      if (alive) fetchCapabilities(false);
    });
    return () => {
      alive = false;
    };
  }, []);

  if (!caps) {
    return (
      <div className="p-6 text-center text-muted text-sm animate-pulse">
        Probing hardware environment...
      </div>
    );
  }

  const gpu = primaryGpu(caps);
  const cudaIssue = gpuPresentButCudaUnavailable(caps);
  const vramTotal = gpu?.total_vram_mb ?? 0;
  const vramUsed = gpu?.used_vram_mb ?? 0;
  const vramFree = gpu?.free_vram_mb ?? 0;
  const vramPct = vramTotal > 0 ? Math.min(100, Math.round((vramUsed / vramTotal) * 100)) : 0;

  const ramTotal = caps.ram.total_mb;
  const ramUsed = caps.ram.used_mb;
  const ramPct = ramTotal > 0 ? Math.min(100, Math.round((ramUsed / ramTotal) * 100)) : 0;

  return (
    <div className={cn("space-y-4", className)}>
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Cpu className="w-5 h-5 text-accent" />
          <h3 className="font-semibold text-ink">Hardware & Acceleration</h3>
        </div>
        <Button
          size="sm"
          variant="outline"
          onClick={() => fetchCapabilities(true)}
          disabled={loading}
          className="text-xs"
        >
          <RefreshCw className={cn("w-3.5 h-3.5 mr-1.5", loading && "animate-spin")} />
          Re-probe Hardware
        </Button>
      </div>

      {cudaIssue ? (
        <div className="p-3.5 rounded-xl bg-warning-soft border border-warning/30 flex items-start gap-3 text-warning text-xs leading-relaxed">
          <AlertTriangle className="w-4 h-4 text-warning shrink-0 mt-0.5" />
          <div>
            <span className="font-semibold block mb-0.5">
              NVIDIA GPU Detected without PyTorch CUDA Support
            </span>
            Your system has an NVIDIA GPU ({gpu?.name}), but the current Python environment does not
            have CUDA acceleration enabled. ASR will run on CPU until CUDA torch is installed.
          </div>
        </div>
      ) : null}

      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        {/* GPU & VRAM Card */}
        <div className="p-4 rounded-xl bg-surface border border-line space-y-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 font-medium text-xs text-ink">
              <Monitor className="w-4 h-4 text-indigo-500" />
              <span>Graphics & Acceleration</span>
            </div>
            <Badge variant={caps.cuda.torch_cuda_available ? "success" : "outline"} size="sm">
              {caps.cuda.torch_cuda_available ? "CUDA Active" : "CPU Fallback"}
            </Badge>
          </div>

          <div className="text-xs space-y-1.5 text-ink-2">
            <div className="flex justify-between">
              <span className="text-muted">Primary GPU:</span>
              <span className="font-medium text-ink">
                {gpu?.name || "None (Software Renderer)"}
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted">Driver / CUDA:</span>
              <span className="font-medium text-ink">
                {caps.cuda.driver_cuda_version
                  ? `v${caps.cuda.driver_cuda_version}`
                  : "Unavailable"}
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted">Vulkan API:</span>
              <span className="font-medium text-ink">
                {caps.vulkan.available ? "Supported" : "Not detected"}
              </span>
            </div>
          </div>

          {vramTotal > 0 ? (
            <div className="space-y-1 pt-1 border-t border-line-soft">
              <div className="flex justify-between text-xs">
                <span className="text-muted">VRAM Allocation ({vramPct}%)</span>
                <span className="font-medium text-ink">
                  {vramUsed} / {vramTotal} MB ({vramFree} MB free)
                </span>
              </div>
              <div className="w-full bg-base-2 h-2 rounded-full overflow-hidden">
                <div
                  className={cn(
                    "h-full rounded-full transition-all duration-300",
                    vramPct > 85 ? "bg-danger" : vramPct > 65 ? "bg-warning" : "bg-indigo-500",
                  )}
                  style={{ width: `${Math.max(4, vramPct)}%` }}
                />
              </div>
            </div>
          ) : null}
        </div>

        {/* CPU & RAM Card */}
        <div className="p-4 rounded-xl bg-surface border border-line space-y-3">
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2 font-medium text-xs text-ink">
              <HardDrive className="w-4 h-4 text-success" />
              <span>CPU & System Memory</span>
            </div>
            <Badge variant="outline" size="sm">
              {caps.cpu.logical_cores} Threads
            </Badge>
          </div>

          <div className="text-xs space-y-1.5 text-ink-2">
            <div className="flex justify-between">
              <span className="text-muted">Processor:</span>
              <span className="font-medium text-ink truncate max-w-[200px]" title={caps.cpu.model}>
                {caps.cpu.model}
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted">Topology:</span>
              <span className="font-medium text-ink">
                {caps.cpu.physical_cores ?? "?"} Physical Cores / {caps.cpu.logical_cores} Logical
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-muted">OS Platform:</span>
              <span className="font-medium text-ink">{caps.os_name}</span>
            </div>
          </div>

          <div className="space-y-1 pt-1 border-t border-line-soft">
            <div className="flex justify-between text-xs">
              <span className="text-muted">System RAM ({ramPct}%)</span>
              <span className="font-medium text-ink">
                {ramUsed} / {ramTotal} MB ({caps.ram.available_mb} MB available)
              </span>
            </div>
            <div className="w-full bg-base-2 h-2 rounded-full overflow-hidden">
              <div
                className={cn(
                  "h-full rounded-full transition-all duration-300",
                  ramPct > 85 ? "bg-danger" : ramPct > 65 ? "bg-warning" : "bg-success",
                )}
                style={{ width: `${Math.max(4, ramPct)}%` }}
              />
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
