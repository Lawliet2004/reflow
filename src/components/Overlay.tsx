import React from "react";
import { AlertCircle, Check } from "lucide-react";
import { AppState, StreamingTranscriptPayload } from "../types";
import { Waveform } from "./Waveform";
import { derivePhase, phaseLabel, BackendStage } from "./hud/stages";

interface OverlayProps {
  appState: AppState;
  transcript: StreamingTranscriptPayload;
  standalone?: boolean;
  extraMessage?: string | null;
  hudTheme?: "dark" | "light" | "auto";
  waveformStyle?: "bars" | "pulse" | "minimal";
  hudScale?: "compact" | "standard" | "large";
  backendStage?: BackendStage | null;
}

/** A compact, non-interactive status capsule. Details stay in the main app. */
export const Overlay: React.FC<OverlayProps> = ({
  appState,
  transcript,
  standalone = false,
  extraMessage = null,
  hudTheme = "dark",
  waveformStyle = "bars",
  hudScale = "standard",
  backendStage = null,
}) => {
  const phase = derivePhase(appState, transcript, extraMessage, backendStage);
  const previewText = transcript.full_text.trim();
  const message = phaseLabel(phase, extraMessage ?? "", previewText);
  const label =
    phase === "error"
      ? "Couldn't finish · open Reflow"
      : phase === "done" && (!extraMessage || /^(inserted|done)[.!]?$/i.test(extraMessage.trim()))
        ? "Done"
        : message;

  if (phase === "hidden") {
    return null;
  }

  const isLightHud =
    hudTheme === "light" ||
    (hudTheme === "auto" &&
      typeof document !== "undefined" &&
      !document.documentElement.classList.contains("dark"));

  const scaleClass =
    hudScale === "compact" ? "hud-scale-compact" : hudScale === "large" ? "hud-scale-large" : "";

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

        <span
          className="hud-label"
          title={phase === "listen" ? "Escape to cancel recording" : message}
        >
          {label}
        </span>

        {phase === "listen" && waveformStyle === "bars" && (
          <span className="hud-wave">
            <Waveform
              level={transcript.audio_level}
              active
              barCount={5}
              height={16}
              tone={isLightHud ? "light" : "dark"}
            />
          </span>
        )}
      </div>
    </div>
  );
};
