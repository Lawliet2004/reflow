import React from "react";
import { AlertCircle, Check } from "lucide-react";
import { AppState, StreamingTranscriptPayload } from "../types";
import { Waveform } from "./Waveform";
import { ModeBadge } from "./hud/ModeBadge";
import { StageRail } from "./hud/StageRail";
import { derivePhase, isNeutralOutcome, phaseLabel, BackendStage } from "./hud/stages";

interface OverlayProps {
  appState: AppState;
  transcript: StreamingTranscriptPayload;
  standalone?: boolean;
  extraMessage?: string | null;
  hudTheme?: "dark" | "light" | "auto";
  waveformStyle?: "bars" | "pulse" | "minimal";
  hudScale?: "compact" | "standard" | "large";
  /** Whether Stage 2 polishing will run for this dictation. */
  polishEnabled?: boolean;
  /**
   * Latest backend pipeline stage (`pipeline:stage` event). Authoritative when
   * set; the transcript string-match stays as the fallback.
   */
  backendStage?: BackendStage | null;
  /** Label of the polish model, for the mode badge tooltip. */
  polishModelLabel?: string;
  /** Show per-stage elapsed time. Driven by the developer_mode setting. */
  showTimings?: boolean;
}

/** Elapsed milliseconds in the current phase, for the developer readout. */
function useElapsed(key: string, running: boolean): number {
  const [elapsed, setElapsed] = React.useState(0);
  const startedAt = React.useRef<number>(0);

  React.useEffect(() => {
    startedAt.current = Date.now();
    setElapsed(0);
    if (!running) return;
    const id = window.setInterval(() => setElapsed(Date.now() - startedAt.current), 100);
    return () => window.clearInterval(id);
  }, [key, running]);

  return elapsed;
}

/**
 * The recording HUD.
 *
 * One capsule that changes contents, deliberately not four separate pills. The
 * previous version rendered listening, transcribing, polishing and inserting as
 * near-identical 56px pills differing only by a single word, which made the
 * pipeline invisible: there was no way to see how far along a dictation was, and
 * "Transcribing" and "Polishing" looked the same. Keeping one container with a
 * fixed height across every active phase also removes the layout jump that used
 * to happen on each transition.
 */
export const Overlay: React.FC<OverlayProps> = ({
  appState,
  transcript,
  standalone = false,
  extraMessage = null,
  hudTheme = "dark",
  waveformStyle = "bars",
  hudScale = "standard",
  polishEnabled = true,
  backendStage = null,
  polishModelLabel,
  showTimings = false,
}) => {
  const phase = derivePhase(appState, transcript, extraMessage, backendStage);
  const previewText = transcript.full_text.trim();
  const label = phaseLabel(phase, extraMessage ?? "", previewText);
  const elapsed = useElapsed(phase, phase !== "done" && phase !== "hidden" && phase !== "error");

  if (!standalone && phase === "hidden") {
    return null;
  }

  const isLightHud =
    hudTheme === "light" ||
    (hudTheme === "auto" &&
      typeof document !== "undefined" &&
      !document.documentElement.classList.contains("dark"));

  const scaleClass =
    hudScale === "compact" ? "hud-scale-compact" : hudScale === "large" ? "hud-scale-large" : "";

  // The live line: partials while working, the result once done.
  const liveText =
    phase === "done" || phase === "error"
      ? previewText
      : transcript.committed_prefix + transcript.mutable_suffix || previewText;

  const neutral = isNeutralOutcome(label);

  return (
    <div
      className={[
        "hud",
        isLightHud ? "hud-light" : "hud-dark",
        scaleClass,
        standalone ? "hud-standalone" : "hud-floating",
      ]
        .filter(Boolean)
        .join(" ")}
      data-phase={phase}
      role="status"
      aria-live="polite"
      aria-atomic="true"
    >
      <div className="hud-row">
        <span className="hud-indicator" data-phase={phase} aria-hidden="true">
          {phase === "error" ? (
            <AlertCircle className="w-4 h-4" />
          ) : phase === "done" ? (
            <Check className="w-3.5 h-3.5 stroke-[3]" />
          ) : phase === "listen" ? (
            <span className="hud-pulse" />
          ) : (
            <span className="hud-spinner" />
          )}
        </span>

        <span className="hud-label" data-neutral={neutral || undefined}>
          {label}
        </span>

        {phase === "listen" && waveformStyle === "bars" && (
          <span className="hud-wave">
            <Waveform
              level={transcript.audio_level}
              active
              barCount={18}
              height={20}
              tone={isLightHud ? "light" : "dark"}
            />
          </span>
        )}

        <span className="hud-spacer" />

        {showTimings && phase !== "hidden" && phase !== "done" && (
          <span className="hud-timing" title="Time in this stage">
            {elapsed} ms
          </span>
        )}

        {phase !== "error" && <StageRail phase={phase} polishEnabled={polishEnabled} />}

        <ModeBadge polishEnabled={polishEnabled} modelLabel={polishModelLabel} />
      </div>

      <p className="hud-text" title={liveText} data-empty={!liveText || undefined}>
        {liveText || (phase === "listen" ? "Listening for speech…" : "\u00a0")}
      </p>
    </div>
  );
};
