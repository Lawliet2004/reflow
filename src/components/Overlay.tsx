import React from "react";
import { AlertCircle, Check } from "lucide-react";
import {
  AppState,
  HudScale,
  HudShape,
  HudStyle,
  HudTheme,
  StreamingTranscriptPayload,
  WaveformStyle,
} from "../types";
import { Waveform } from "./Waveform";
import { derivePhase, phaseLabel, BackendStage } from "./hud/stages";

interface OverlayProps {
  appState: AppState;
  transcript: StreamingTranscriptPayload;
  standalone?: boolean;
  exiting?: boolean;
  extraMessage?: string | null;
  hudTheme?: HudTheme;
  waveformStyle?: WaveformStyle;
  hudScale?: HudScale;
  hudShape?: HudShape;
  hudStyle?: HudStyle;
  /** Background opacity, 0.6–1. */
  hudOpacity?: number;
  backendStage?: BackendStage | null;
}

const WAVE_HEIGHT: Record<HudScale, number> = { compact: 14, standard: 16, large: 20 };
const SCALE_CLASS: Record<HudScale, string> = {
  compact: "hud-scale-compact",
  standard: "",
  large: "hud-scale-large",
};
const CAPSULE_BARS = [0.35, 0.65, 1, 0.8, 0.55, 0.35, 0.15];

/** Audio-driven while listening; a gentle travelling wave during processing. */
const CapsuleWave: React.FC<{ level: number; listening: boolean }> = ({ level, listening }) => {
  const amplitude = Number.isFinite(level) ? Math.max(0, Math.min(1, level * 4)) : 0;
  return (
    <span className="capsule-wave" data-active={listening} aria-hidden="true">
      {CAPSULE_BARS.map((weight, i) => (
        <span
          key={i}
          className="capsule-wave-bar"
          style={
            {
              height: `${3 + 17 * weight * (listening ? amplitude : 1)}px`,
              "--bar-weight": weight,
              animationDelay: `${i * -110}ms`,
            } as React.CSSProperties
          }
        />
      ))}
    </span>
  );
};

/** A compact, non-interactive status capsule. Details stay in the main app. */
export const Overlay: React.FC<OverlayProps> = ({
  appState,
  transcript,
  standalone = false,
  exiting = false,
  extraMessage = null,
  hudTheme = "dark",
  waveformStyle = "bars",
  hudScale = "standard",
  hudShape = "pill",
  hudStyle = "status",
  hudOpacity = 0.96,
  backendStage = null,
}) => {
  const phase = derivePhase(appState, transcript, extraMessage, backendStage);
  const previewText = transcript.full_text.trim();
  const message = phaseLabel(phase, extraMessage ?? "", previewText);
  let label = message;
  if (phase === "error") {
    label = "Couldn't finish · open Reflow";
  } else if (
    phase === "done" &&
    (!extraMessage || /^(inserted|done)[.!]?$/i.test(extraMessage.trim()))
  ) {
    label = "Done";
  }

  if (phase === "hidden") {
    return null;
  }

  const isLightHud =
    hudTheme === "light" ||
    (hudTheme === "auto" &&
      typeof document !== "undefined" &&
      !document.documentElement.classList.contains("dark"));

  // Minimal shows only the dot and level while listening; status words return after.
  const minimal = waveformStyle === "minimal" && phase === "listen";
  const waveformPill = hudStyle === "waveform";
  // An unpolished insertion is still a completed paste. Keep the waveform's
  // checkmark and geometry; reserve expansion for messages requiring attention.
  const compactCompletion = label === "Done" || /^Inserted \(unpolished\)[.!]?$/i.test(label);
  const showMessage =
    waveformPill && (phase === "error" || (phase === "done" && !compactCompletion));
  const scaleClass = SCALE_CLASS[hudScale];

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
      data-exiting={exiting || undefined}
      data-shape={hudShape}
      data-style={hudStyle}
      data-message={showMessage || undefined}
      data-minimal={(!waveformPill && minimal) || undefined}
      style={{ "--hud-alpha": hudOpacity } as React.CSSProperties}
      role="status"
      aria-live="polite"
      aria-atomic="true"
      title={waveformPill ? (phase === "listen" ? "Listening" : label) : undefined}
    >
      {waveformPill && !showMessage ? (
        <div className="hud-capsule-row">
          <CapsuleWave level={transcript.audio_level} listening={phase === "listen"} />
          <Check className="capsule-check" strokeWidth={2.5} aria-hidden="true" />
          <span className="sr-only">{label}</span>
        </div>
      ) : (
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

          <span
            key={label}
            className={minimal ? "sr-only" : "hud-label"}
            title={phase === "listen" ? "Listening" : message}
          >
            {label}
          </span>

          {phase === "listen" && waveformStyle !== "pulse" && (
            <span className="hud-wave">
              <Waveform
                level={transcript.audio_level}
                active
                barCount={minimal ? 9 : 5}
                height={WAVE_HEIGHT[hudScale]}
                tone={isLightHud ? "light" : "dark"}
              />
            </span>
          )}
        </div>
      )}
    </div>
  );
};
