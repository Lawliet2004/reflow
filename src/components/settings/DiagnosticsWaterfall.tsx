import React, { useEffect, useState } from "react";
import { tauriApi as api } from "../../services/tauriApi";
import { LatencyReport, SystemMetrics } from "../../types";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { cn } from "../../utils/cn";
import { Activity, Clock, HardDrive, RefreshCw, RotateCcw } from "lucide-react";

export function DiagnosticsWaterfall({ className }: { className?: string }) {
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
    fetchDiagnostics();
    const interval = setInterval(fetchDiagnostics, 3000);
    return () => clearInterval(interval);
  }, []);

  const last = report?.last;
  const p = report?.percentiles;
  const totalReleaseMs = last ? Math.max(1, last.release_to_inserted_ms) : 1;

  const asrPct = last ? Math.min(100, (last.speech_end_to_final_ms / totalReleaseMs) * 100) : 0;
  const formatPct = last ? Math.min(100, (last.formatting_ms / totalReleaseMs) * 100) : 0;
  const llmPct = last ? Math.min(100, (last.rewrite_ms / totalReleaseMs) * 100) : 0;
  const injectPct = last ? Math.min(100, (last.final_to_injection_ms / totalReleaseMs) * 100) : 0;

  return (
    <div className={cn("space-y-6", className)}>
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Activity className="w-5 h-5 text-sky-500" />
          <h3 className="font-semibold text-slate-900 dark:text-slate-100">
            Live Latency & Diagnostics
          </h3>
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
      <div className="p-4 rounded-xl bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 space-y-4">
        <div className="flex items-center justify-between text-xs font-medium text-slate-700 dark:text-slate-300">
          <span className="flex items-center gap-1.5">
            <Clock className="w-4 h-4 text-sky-500" />
            Last Dictation Timeline ({last?.release_to_inserted_ms ?? 0} ms)
          </span>
          <Badge variant={last?.rewrite_applied ? "primary" : "outline"} size="sm">
            {last?.rewrite_applied ? "LLM Applied" : "Stage 1 Direct"}
          </Badge>
        </div>

        {/* Phase Bar */}
        <div className="h-4 w-full bg-slate-100 dark:bg-slate-800 rounded-full overflow-hidden flex">
          {asrPct > 0 ? (
            <div
              style={{ width: `${asrPct}%` }}
              className="bg-sky-500 h-full transition-all duration-300"
              title={`ASR Inference: ${last?.speech_end_to_final_ms}ms`}
            />
          ) : null}
          {formatPct > 0 ? (
            <div
              style={{ width: `${formatPct}%` }}
              className="bg-teal-500 h-full transition-all duration-300"
              title={`Formatting: ${last?.formatting_ms}ms`}
            />
          ) : null}
          {llmPct > 0 ? (
            <div
              style={{ width: `${llmPct}%` }}
              className="bg-indigo-500 h-full transition-all duration-300"
              title={`LLM Rewrite: ${last?.rewrite_ms}ms`}
            />
          ) : null}
          {injectPct > 0 ? (
            <div
              style={{ width: `${injectPct}%` }}
              className="bg-emerald-500 h-full transition-all duration-300"
              title={`Injection: ${last?.final_to_injection_ms}ms`}
            />
          ) : null}
        </div>

        {/* Phase legend */}
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 text-xs pt-1">
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-full bg-sky-500" />
            <span className="text-slate-500 dark:text-slate-400">ASR:</span>
            <span className="font-semibold text-slate-800 dark:text-slate-200">
              {last?.speech_end_to_final_ms ?? 0}ms
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-full bg-teal-500" />
            <span className="text-slate-500 dark:text-slate-400">Format:</span>
            <span className="font-semibold text-slate-800 dark:text-slate-200">
              {last?.formatting_ms ?? 0}ms
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-full bg-indigo-500" />
            <span className="text-slate-500 dark:text-slate-400">LLM:</span>
            <span className="font-semibold text-slate-800 dark:text-slate-200">
              {last?.rewrite_ms ?? 0}ms
            </span>
          </div>
          <div className="flex items-center gap-1.5">
            <span className="w-2.5 h-2.5 rounded-full bg-emerald-500" />
            <span className="text-slate-500 dark:text-slate-400">Inject:</span>
            <span className="font-semibold text-slate-800 dark:text-slate-200">
              {last?.final_to_injection_ms ?? 0}ms
            </span>
          </div>
        </div>
      </div>

      {/* Percentiles & RTF */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <div className="p-3 rounded-xl bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 text-xs">
          <span className="text-slate-400 block mb-1">Latency (p50)</span>
          <span className="text-lg font-bold text-slate-900 dark:text-slate-100">
            {p?.release_to_inserted_p50_ms ?? 0} ms
          </span>
        </div>
        <div className="p-3 rounded-xl bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 text-xs">
          <span className="text-slate-400 block mb-1">Latency (p95)</span>
          <span className="text-lg font-bold text-slate-900 dark:text-slate-100">
            {p?.release_to_inserted_p95_ms ?? 0} ms
          </span>
        </div>
        <div className="p-3 rounded-xl bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 text-xs">
          <span className="text-slate-400 block mb-1">Real-Time Factor (RTF)</span>
          <span className="text-lg font-bold text-slate-900 dark:text-slate-100">
            {last?.rtf ? last.rtf.toFixed(2) : "0.00"}x
          </span>
        </div>
        <div className="p-3 rounded-xl bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 text-xs">
          <span className="text-slate-400 block mb-1">LLM Polish Rate</span>
          <span className="text-lg font-bold text-slate-900 dark:text-slate-100">
            {p ? Math.round(p.llm_applied_rate * 100) : 0}%
          </span>
        </div>
      </div>

      {/* Memory Pressure Breakdown */}
      {metrics ? (
        <div className="p-4 rounded-xl bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 space-y-3">
          <div className="flex items-center gap-2 text-xs font-semibold text-slate-800 dark:text-slate-200">
            <HardDrive className="w-4 h-4 text-emerald-500" />
            <span>Hardware Memory Allocation</span>
          </div>
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-3 text-xs">
            <div className="p-2.5 rounded-lg bg-slate-50 dark:bg-slate-800/50">
              <span className="text-slate-400 block">App RAM</span>
              <span className="font-semibold text-slate-800 dark:text-slate-200">
                {metrics.app_ram_mb} MB
              </span>
            </div>
            <div className="p-2.5 rounded-lg bg-slate-50 dark:bg-slate-800/50">
              <span className="text-slate-400 block">Model RAM</span>
              <span className="font-semibold text-slate-800 dark:text-slate-200">
                {metrics.model_ram_mb} MB
              </span>
            </div>
            <div className="p-2.5 rounded-lg bg-slate-50 dark:bg-slate-800/50">
              <span className="text-slate-400 block">
                {metrics.gpu_present ? `VRAM (${metrics.gpu_name})` : "System RAM Total"}
              </span>
              <span className="font-semibold text-slate-800 dark:text-slate-200">
                {metrics.gpu_present
                  ? `${metrics.vram_mb} / ${metrics.total_vram_mb} MB`
                  : `${metrics.total_ram_mb} MB`}
              </span>
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
