import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { LatencyWaterfall, LATENCY_STAGES, formatMs } from "./LatencyWaterfall";
import { emptyLatencyMetrics, emptyLatencyPercentiles } from "../services/tauriApi";
import type { LatencyMetrics, LatencyPercentiles } from "../types";

function populatedMetrics(): LatencyMetrics {
  return {
    ...emptyLatencyMetrics(),
    hotkey_to_recording_ms: 120,
    recording_to_first_audio_ms: 30,
    audio_to_first_partial_ms: 250,
    speech_end_to_final_ms: 300,
    llm_startup_ms: 5,
    formatting_ms: 5,
    rewrite_ms: 490,
    rewrite_applied: true,
    final_to_injection_ms: 550,
    release_to_inserted_ms: 850,
    total_duration_ms: 5850,
    audio_duration_ms: 4900,
    rtf: 0.061,
    segments: [
      {
        sequence_id: 0,
        submitted_at_ms: 5020,
        completed_at_ms: 5300,
        compute_ms: 280,
        audio_ms: 4900,
        rtf: 0.057,
      },
    ],
  };
}

function populatedPercentiles(): LatencyPercentiles {
  return {
    ...emptyLatencyPercentiles(),
    samples: 5,
    hotkey_to_recording_p50_ms: 110,
    hotkey_to_recording_p95_ms: 240,
    speech_end_to_final_p50_ms: 300,
    speech_end_to_final_p95_ms: 480,
    rewrite_p50_ms: 500,
    rewrite_p95_ms: 900,
    release_to_inserted_p50_ms: 860,
    release_to_inserted_p95_ms: 1400,
    rtf_p50: 0.06,
    rtf_p95: 0.09,
    llm_applied_rate: 0.8,
  };
}

describe("formatMs", () => {
  it("switches to seconds above a second", () => {
    expect(formatMs(0)).toBe("0 ms");
    expect(formatMs(999)).toBe("999 ms");
    expect(formatMs(1000)).toBe("1.00 s");
    expect(formatMs(5850)).toBe("5.85 s");
  });
});

describe("LatencyWaterfall", () => {
  it("renders a row for every stage", () => {
    render(<LatencyWaterfall metrics={populatedMetrics()} />);
    for (const stage of LATENCY_STAGES) {
      expect(
        screen.getByTestId(String(stage.key)),
        `${String(stage.key)} row is missing`,
      ).toBeInTheDocument();
    }
  });

  it("shows each stage's measured value", () => {
    render(<LatencyWaterfall metrics={populatedMetrics()} />);
    expect(screen.getByTestId("hotkey_to_recording_ms")).toHaveTextContent("120 ms");
    expect(screen.getByTestId("audio_to_first_partial_ms")).toHaveTextContent("250 ms");
    expect(screen.getByTestId("rewrite_ms")).toHaveTextContent("490 ms");
    expect(screen.getByTestId("release_to_inserted_ms")).toHaveTextContent("850 ms");
  });

  it("distinguishes a stage that did not run from one that is not instrumented", () => {
    const metrics = { ...populatedMetrics(), rewrite_ms: 0, rewrite_applied: false };
    render(<LatencyWaterfall metrics={metrics} />);
    const cell = screen.getByTestId("rewrite_ms");
    expect(cell).toHaveTextContent("—");
    expect(cell.querySelector("[title]")).toHaveAttribute("title", "no rewrite ran");
  });

  it("reports total, audio duration and RTF", () => {
    render(<LatencyWaterfall metrics={populatedMetrics()} />);
    const header = screen.getByRole("heading", { name: "Latency waterfall" }).parentElement!;
    expect(header).toHaveTextContent("total 5.85 s");
    expect(header).toHaveTextContent("audio 4.90 s");
    expect(header).toHaveTextContent("RTF 0.061");
  });

  it("says plainly when nothing has been measured", () => {
    render(<LatencyWaterfall metrics={emptyLatencyMetrics()} />);
    expect(screen.getByText("no dictation recorded yet")).toBeInTheDocument();
    // Every stage still renders, so a missing stage stays visible as missing.
    expect(screen.getAllByText("—").length).toBe(LATENCY_STAGES.length);
  });

  it("renders per-segment timings", () => {
    render(<LatencyWaterfall metrics={populatedMetrics()} />);
    expect(screen.getByRole("heading", { name: "ASR segments" })).toBeInTheDocument();
    expect(
      screen.getByText(/#0 · 4\.90 s audio · 280 ms compute · RTF 0\.057/),
    ).toBeInTheDocument();
  });

  it("renders rolling percentiles and the LLM-applied rate", () => {
    render(<LatencyWaterfall metrics={populatedMetrics()} percentiles={populatedPercentiles()} />);
    expect(
      screen.getByRole("heading", { name: /Rolling p50 \/ p95 over 5 dictations/ }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("p-hotkey")).toHaveTextContent("110 ms / 240 ms");
    expect(screen.getByTestId("p-rewrite")).toHaveTextContent("500 ms / 900 ms");
    expect(screen.getByTestId("p-rtf")).toHaveTextContent("0.060 / 0.090");
    // The Balanced preset's acceptance criterion is >= 90%.
    expect(screen.getByTestId("p-applied")).toHaveTextContent("80%");
  });

  it("omits the percentile block until there is a sample", () => {
    render(
      <LatencyWaterfall metrics={populatedMetrics()} percentiles={emptyLatencyPercentiles()} />,
    );
    expect(screen.queryByText(/Rolling p50/)).not.toBeInTheDocument();
  });
});

describe("latency report shape", () => {
  /**
   * Guards the JSON contract that `reflow --latency-json` prints and that
   * scripts diff between runs. A field rename on the Rust side has to break
   * this test rather than silently produce `undefined` in a report.
   */
  it("matches the documented snapshot", () => {
    const report = {
      last: { ...emptyLatencyMetrics(), last_updated: "<iso>" },
      percentiles: emptyLatencyPercentiles(),
      recent: [] as LatencyMetrics[],
    };
    expect(report).toMatchInlineSnapshot(`
      {
        "last": {
          "audio_duration_ms": 0,
          "audio_to_first_partial_ms": 0,
          "final_to_injection_ms": 0,
          "formatting_ms": 0,
          "hotkey_to_recording_ms": 0,
          "last_updated": "<iso>",
          "llm_startup_ms": 0,
          "recording_to_first_audio_ms": 0,
          "release_to_inserted_ms": 0,
          "rewrite_applied": false,
          "rewrite_ms": 0,
          "rtf": 0,
          "segments": [],
          "speech_end_to_final_ms": 0,
          "total_duration_ms": 0,
        },
        "percentiles": {
          "hotkey_to_recording_p50_ms": 0,
          "hotkey_to_recording_p95_ms": 0,
          "llm_applied_rate": 0,
          "release_to_inserted_p50_ms": 0,
          "release_to_inserted_p95_ms": 0,
          "rewrite_p50_ms": 0,
          "rewrite_p95_ms": 0,
          "rtf_p50": 0,
          "rtf_p95": 0,
          "samples": 0,
          "speech_end_to_final_p50_ms": 0,
          "speech_end_to_final_p95_ms": 0,
        },
        "recent": [],
      }
    `);
  });
});
