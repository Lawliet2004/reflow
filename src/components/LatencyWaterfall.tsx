import React from "react";
import type { LatencyMetrics, LatencyPercentiles } from "../types";

/**
 * Milestone 0 / Task 5: make the repaired telemetry visible during
 * development without opening a log file.
 *
 * Every stage is listed even when it measured zero, because "this stage did not
 * run" and "this stage is not instrumented" used to be indistinguishable and
 * that is precisely what made the latency numbers unfalsifiable.
 */

export interface LatencyStage {
  key: keyof LatencyMetrics;
  label: string;
  /** Shown when the stage reports zero, to explain why. */
  zeroHint: string;
}

/**
 * Ordered so the rows read as the actual sequence of events. `formatting_ms`
 * and `rewrite_ms` are siblings inside `final_to_injection_ms`, so they are
 * marked as nested rather than summed alongside it.
 */
export const LATENCY_STAGES: readonly LatencyStage[] = [
  {
    key: "hotkey_to_recording_ms",
    label: "Hotkey → recording",
    zeroHint: "no press timestamp captured",
  },
  {
    key: "recording_to_first_audio_ms",
    label: "Recording → first audio",
    zeroHint: "no audio arrived",
  },
  {
    key: "audio_to_first_partial_ms",
    label: "Audio → first partial",
    zeroHint: "no partial emitted (whole-utterance mode)",
  },
  {
    key: "speech_end_to_final_ms",
    label: "Release → final ASR",
    zeroHint: "ASR did not run",
  },
  {
    key: "llm_startup_ms",
    label: "· LLM runtime ready",
    zeroHint: "runtime already warm",
  },
  {
    key: "formatting_ms",
    label: "· Deterministic cleanup",
    zeroHint: "under 1 ms",
  },
  { key: "rewrite_ms", label: "· LLM refinement", zeroHint: "no rewrite ran" },
  {
    key: "final_to_injection_ms",
    label: "Final ASR → inserted",
    zeroHint: "nothing was inserted",
  },
  {
    key: "release_to_inserted_ms",
    label: "Release → inserted",
    zeroHint: "nothing was inserted",
  },
] as const;

export function formatMs(value: number): string {
  if (value >= 1000) return `${(value / 1000).toFixed(2)} s`;
  return `${Math.round(value)} ms`;
}

/** Widest bar in the chart, so rows are comparable against each other. */
function chartMax(metrics: LatencyMetrics): number {
  const values = LATENCY_STAGES.map((s) => Number(metrics[s.key] ?? 0));
  return Math.max(1, ...values);
}

interface WaterfallProps {
  metrics: LatencyMetrics;
  percentiles?: LatencyPercentiles | null;
}

export const LatencyWaterfall: React.FC<WaterfallProps> = ({ metrics, percentiles }) => {
  const max = chartMax(metrics);
  const hasMeasurement = metrics.total_duration_ms > 0;

  return (
    <section aria-label="Latency waterfall" className="text-xs">
      {hasMeasurement && (
        <p className="text-muted mb-3">
          Queue peak: {metrics.peak_audio_queue_chunks ?? 0} chunks · Dropped:{" "}
          {metrics.dropped_audio_chunks ?? 0} · Failed transfers: {metrics.failed_audio_pushes ?? 0}{" "}
          · Writing runtime:{" "}
          {metrics.rewrite_ms ? (metrics.llm_was_warm ? "warm" : "cold start") : "not used"}
        </p>
      )}
      {metrics.llm_generation && (
        <p className="text-muted mb-3">
          Writing: {metrics.llm_generation.prompt_tokens} input tokens ·{" "}
          {metrics.llm_generation.output_tokens} output tokens
          {metrics.llm_generation.prompt_ms != null &&
            ` · Prompt ${Math.round(metrics.llm_generation.prompt_ms)} ms`}
          {metrics.llm_generation.decode_ms != null &&
            ` · Decode ${Math.round(metrics.llm_generation.decode_ms)} ms`}
          {Boolean(metrics.llm_generation.decode_ms) &&
            ` · ${((metrics.llm_generation.output_tokens * 1000) / metrics.llm_generation.decode_ms!).toFixed(1)} tokens/s`}
        </p>
      )}
      <header className="flex items-baseline justify-between gap-3">
        <h3 className="font-medium">Latency waterfall</h3>
        {hasMeasurement ? (
          <span className="tabular-nums opacity-70">
            total {formatMs(metrics.total_duration_ms)} · audio{" "}
            {formatMs(metrics.audio_duration_ms)} · RTF {metrics.rtf.toFixed(3)}
          </span>
        ) : (
          <span className="opacity-70">no dictation recorded yet</span>
        )}
      </header>

      <ul className="mt-2 space-y-1">
        {LATENCY_STAGES.map((stage) => {
          const value = Number(metrics[stage.key] ?? 0);
          const pct = (value / max) * 100;
          const nested = stage.label.startsWith("·");
          return (
            <li
              key={String(stage.key)}
              className="grid grid-cols-[minmax(0,11rem)_1fr_5rem] items-center gap-2"
            >
              <span className={nested ? "pl-3 opacity-80" : ""}>{stage.label}</span>
              <span
                className="h-1.5 rounded-full bg-current/10"
                role="presentation"
                data-testid={`bar-${String(stage.key)}`}
              >
                <span
                  className="block h-full rounded-full bg-current/60"
                  style={{ width: `${pct}%` }}
                />
              </span>
              <span className="text-right tabular-nums" data-testid={String(stage.key)}>
                {value > 0 ? (
                  formatMs(value)
                ) : (
                  <span title={stage.zeroHint} className="opacity-50">
                    —
                  </span>
                )}
              </span>
            </li>
          );
        })}
      </ul>

      {metrics.segments.length > 0 && (
        <div className="mt-3">
          <h4 className="font-medium">ASR segments</h4>
          <ul className="mt-1 space-y-0.5">
            {metrics.segments.map((seg) => (
              <li key={seg.sequence_id} className="tabular-nums opacity-80">
                #{seg.sequence_id} · {formatMs(seg.audio_ms)} audio ·{" "}
                {seg.compute_ms === null ? "in flight" : `${formatMs(seg.compute_ms)} compute`}
                {seg.rtf !== null && ` · RTF ${seg.rtf.toFixed(3)}`}
              </li>
            ))}
          </ul>
        </div>
      )}

      {percentiles && percentiles.samples > 0 && (
        <div className="mt-3">
          <h4 className="font-medium">
            Rolling p50 / p95 over {percentiles.samples}{" "}
            {percentiles.samples === 1 ? "dictation" : "dictations"}
          </h4>
          <dl className="mt-1 grid grid-cols-2 gap-x-4 gap-y-0.5">
            <dt>Hotkey → recording</dt>
            <dd className="text-right tabular-nums" data-testid="p-hotkey">
              {formatMs(percentiles.hotkey_to_recording_p50_ms)} /{" "}
              {formatMs(percentiles.hotkey_to_recording_p95_ms)}
            </dd>
            <dt>Release → final ASR</dt>
            <dd className="text-right tabular-nums" data-testid="p-final">
              {formatMs(percentiles.speech_end_to_final_p50_ms)} /{" "}
              {formatMs(percentiles.speech_end_to_final_p95_ms)}
            </dd>
            <dt>LLM refinement</dt>
            <dd className="text-right tabular-nums" data-testid="p-rewrite">
              {formatMs(percentiles.rewrite_p50_ms)} / {formatMs(percentiles.rewrite_p95_ms)}
            </dd>
            <dt>Release → inserted</dt>
            <dd className="text-right tabular-nums" data-testid="p-inserted">
              {formatMs(percentiles.release_to_inserted_p50_ms)} /{" "}
              {formatMs(percentiles.release_to_inserted_p95_ms)}
            </dd>
            <dt>RTF</dt>
            <dd className="text-right tabular-nums" data-testid="p-rtf">
              {percentiles.rtf_p50.toFixed(3)} / {percentiles.rtf_p95.toFixed(3)}
            </dd>
            <dt>LLM applied</dt>
            <dd className="text-right tabular-nums" data-testid="p-applied">
              {(percentiles.llm_applied_rate * 100).toFixed(0)}%
            </dd>
          </dl>
        </div>
      )}
    </section>
  );
};
