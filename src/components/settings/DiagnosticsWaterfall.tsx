import React, { useEffect, useState } from "react";
import { tauriApi as api } from "../../services/tauriApi";
import { LatencyReport, SystemMetrics } from "../../types";
import { useViewVisibility } from "../../hooks/useViewVisibility";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { cn } from "../../utils/cn";
import { Activity, Clock, HardDrive, RefreshCw, RotateCcw } from "lucide-react";

export function DiagnosticsWaterfall({ className }: { className?: string }) {
  const visible = useViewVisibility();
  const [report, setReport] = useState<LatencyReport | null>(null);
  const [metrics, setMetrics] = useState<SystemMetrics | null>(null);
  const [loading, setLoading] = useState(false);

  const fetchDiagnostics = async () => {
    setLoading(true);
    try {
      const [rep, sys] = await Promise.all([api.getLatencyReport(), api.getSystemMetrics()]);
      setReport(rep);
      setMetrics(sys);
    } catch (e) {
      console.error("failed to load diagnostics report", e);
    } finally {
      setLoading(false);
    }
  };

  const handleReset = async () => {
    const res = await api.resetLatencyHistory();
    setReport(res);
  };

  useEffect(() => {
    if (!visible) return;
    let alive = true;
    queueMicrotask(() => {
      if (alive) fetchDiagnostics();
    });
    const interval = setInterval(fetchDiagnostics, 3000);
    return () => {
      alive = false;
      clearInterval(interval);
    };
  }, [visible]);

  const last = report?.last;
  const p = report?.percentiles;
  const totalReleaseMs = last ? Math.max(1, last.release_to_inserted_ms) : 1;

  const phasePct = (ms: number) => (last ? Math.min(100, (ms / totalReleaseMs) * 100) : 0);
  const phases = [
    {
      id: "asr",
      title: "ASR Inference",
      legend: "ASR",
      color: "bg-accent",
      ms: last?.speech_end_to_final_ms ?? 0,
    },
    {
      id: "format",
      title: "Formatting",
      legend: "Format",
      color: "bg-teal-500",
      ms: last?.formatting_ms ?? 0,
    },
    {
      id: "llm",
      title: "LLM Rewrite",
      legend: "LLM",
      color: "bg-indigo-500",
      ms: last?.rewrite_ms ?? 0,
    },
    {
      id: "inject",
      title: "Injection",
      legend: "Inject",
      color: "bg-success",
      ms: last?.final_to_injection_ms ?? 0,
    },
  ].map((phase) => ({ ...phase, pct: phasePct(phase.ms) }));

  return (
    <div className={cn("space-y-6", className)}>
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Activity className="w-5 h-5 text-accent" />
          <h3 className="font-semibold text-ink">Live Latency & Diagnostics</h3>
        </div>
        <div className="flex items-center gap-2">
          <Button size="sm" variant="outline" onClick={handleReset} className="text-xs">
            <RotateCcw className="w-3.5 h-3.5 mr-1" /> Reset History
          </Button>
          <Button
            size="sm"
            variant="ghost"
            onClick={fetchDiagnostics}
            disabled={loading}
            className="text-xs"
          >
            <RefreshCw className={cn("w-3.5 h-3.5", loading && "animate-spin")} />
          </Button>
        </div>
      </div>

      {/* Latency Waterfall */}
      <div className="p-4 rounded-xl bg-surface border border-line space-y-4">
        <div className="flex items-center justify-between text-xs font-medium text-ink-2">
          <span className="flex items-center gap-1.5">
            <Clock className="w-4 h-4 text-accent" />
            Last Dictation Timeline ({last?.release_to_inserted_ms ?? 0} ms)
          </span>
          <Badge variant={last?.rewrite_applied ? "primary" : "outline"} size="sm">
            {last?.rewrite_applied ? "LLM Applied" : "Stage 1 Direct"}
          </Badge>
        </div>

        {last?.rewrite_fallback_reason && (
          <p className="text-xs text-muted">
            Cleanup fallback: <code>{last.rewrite_fallback_reason}</code>
          </p>
        )}

        {/* Phase Bar */}
        <div className="h-4 w-full bg-base-2 rounded-full overflow-hidden flex">
          {phases.map(
            (phase) =>
              phase.pct > 0 && (
                <div
                  key={phase.id}
                  style={{ width: `${phase.pct}%` }}
                  className={`${phase.color} h-full transition-all duration-300`}
                  title={`${phase.title}: ${phase.ms}ms`}
                />
              ),
          )}
        </div>

        {/* Phase legend */}
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 text-xs pt-1">
          {phases.map((phase) => (
            <div key={phase.id} className="flex items-center gap-1.5">
              <span className={`w-2.5 h-2.5 rounded-full ${phase.color}`} />
              <span className="text-muted">{phase.legend}:</span>
              <span className="font-semibold text-ink">{phase.ms}ms</span>
            </div>
          ))}
        </div>
      </div>

      {/* Percentiles & RTF */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <div className="p-3 rounded-xl bg-surface border border-line text-xs">
          <span className="text-muted block mb-1">Latency (p50)</span>
          <span className="text-lg font-bold text-ink">
            {p?.release_to_inserted_p50_ms ?? 0} ms
          </span>
        </div>
        <div className="p-3 rounded-xl bg-surface border border-line text-xs">
          <span className="text-muted block mb-1">Latency (p95)</span>
          <span className="text-lg font-bold text-ink">
            {p?.release_to_inserted_p95_ms ?? 0} ms
          </span>
        </div>
        <div className="p-3 rounded-xl bg-surface border border-line text-xs">
          <span className="text-muted block mb-1">Real-Time Factor (RTF)</span>
          <span className="text-lg font-bold text-ink">
            {last?.rtf ? last.rtf.toFixed(2) : "0.00"}x
          </span>
        </div>
        <div className="p-3 rounded-xl bg-surface border border-line text-xs">
          <span className="text-muted block mb-1">LLM Polish Rate</span>
          <span className="text-lg font-bold text-ink">
            {p ? Math.round(p.llm_applied_rate * 100) : 0}%
          </span>
        </div>
      </div>

      {/* Memory Pressure Breakdown */}
      {metrics && (
        <div className="p-4 rounded-xl bg-surface border border-line space-y-3">
          <div className="flex items-center gap-2 text-xs font-semibold text-ink">
            <HardDrive className="w-4 h-4 text-success" />
            <span>Hardware Memory Allocation</span>
          </div>
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 text-xs">
            <div className="p-2.5 rounded-lg bg-surface-2">
              <span className="text-muted block">App RAM</span>
              <span className="font-semibold text-ink">{metrics.app_ram_mb} MB</span>
            </div>
            <div className="p-2.5 rounded-lg bg-surface-2">
              <span className="text-muted block">Model RAM</span>
              <span className="font-semibold text-ink">{metrics.model_ram_mb} MB</span>
            </div>
            <div className="p-2.5 rounded-lg bg-surface-2">
              <span className="text-muted block">
                {metrics.gpu_present ? `VRAM (${metrics.gpu_name})` : "System RAM Total"}
              </span>
              <span className="font-semibold text-ink">
                {metrics.gpu_present
                  ? `${metrics.vram_mb} / ${metrics.total_vram_mb} MB`
                  : `${metrics.total_ram_mb} MB`}
              </span>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
