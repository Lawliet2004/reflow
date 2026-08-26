import React from "react";
import { Check, AlertCircle } from "lucide-react";
import { AppState, StreamingTranscriptPayload } from "../types";
import { Waveform } from "./Waveform";

interface OverlayProps {
  appState: AppState;
  transcript: StreamingTranscriptPayload;
  standalone?: boolean;
  extraMessage?: string | null;
  hudTheme?: "dark" | "light" | "auto";
  waveformStyle?: "bars" | "pulse" | "minimal";
  hudScale?: "compact" | "standard" | "large";
}

function isPolishing(transcript: StreamingTranscriptPayload, extraMessage?: string | null) {
  const stage = (transcript.stage ?? "").toLowerCase();
  const extra = (extraMessage ?? "").toLowerCase();
  return stage === "polishing" || extra === "polishing" || extra.includes("polish");
}

function isNeutralStatus(label: string): boolean {
  return label === "No speech detected" || label.startsWith("No mic") || label === "Error";
}

function doneLabel(extraMessage: string | null, fullText: string): string {
  const msg = (extraMessage ?? "").trim();
  if (msg === "No speech" || msg === "No speech detected") return "No speech detected";
  if (msg) return msg;
  if (!fullText.trim()) return "No speech detected";
  return "Inserted";
}

const ListeningPulse: React.FC<{ tone?: "dark" | "light" }> = () => (
  <span className="relative flex items-center justify-center w-3 h-3">
    <span className="absolute inset-0 rounded-full bg-accent/40 animate-ping" />
    <span className="relative w-2.5 h-2.5 rounded-full bg-accent ring-4 ring-accent/20" />
  </span>
);

/**
 * Recording HUD — listening pill, then a compact preview card.
 */
export const Overlay: React.FC<OverlayProps> = ({
  appState,
  transcript,
  standalone = false,
  extraMessage = null,
  hudTheme = "dark",
  waveformStyle = "bars",
  hudScale = "standard",
}) => {
  const isRecording = appState === "RECORDING";
  const isProcessing = appState === "PROCESSING";
  const isInjecting = appState === "INJECTING";
  const isError = appState === "ERROR";
  const isDone = appState === "IDLE" || appState === "READY";
  const previewText = transcript.full_text.trim();

  if (!standalone && isDone && !extraMessage) {
    return null;
  }

  const polishing = isProcessing && isPolishing(transcript, extraMessage);
  const statusLabel = doneLabel(extraMessage, previewText);
  const showPreviewLine = isDone && previewText.length > 0;
  const neutral = isNeutralStatus(statusLabel);

  const isLightHud =
    hudTheme === "light" ||
    (hudTheme === "auto" &&
      typeof document !== "undefined" &&
      !document.documentElement.classList.contains("dark"));
  const hudContainerClass = isLightHud ? "overlay-hud-light" : "overlay-hud";

  const scaleClass =
    hudScale === "compact"
      ? "w-[360px]"
      : hudScale === "large"
      ? "w-[480px]"
      : "w-[420px]";

  return (
    <div
      className={
        standalone
          ? "w-full h-full flex items-stretch select-none"
          : `fixed bottom-8 left-1/2 -translate-x-1/2 z-50 ${scaleClass} animate-fade-rise select-none`
      }
    >
      {isRecording ? (
        <div className={`${hudContainerClass} overlay-hud-pill w-fit mx-auto max-w-full h-[56px] flex items-center gap-3 px-5`}>
          <ListeningPulse tone={isLightHud ? "light" : "dark"} />
          {waveformStyle === "bars" && (
            <Waveform
              level={transcript.audio_level}
              active
              barCount={22}
              height={22}
              tone={isLightHud ? "light" : "dark"}
            />
          )}
          {waveformStyle === "pulse" && (
            <div className="flex items-center justify-center">
              <span className="w-2.5 h-2.5 rounded-full bg-accent animate-ping" />
            </div>
          )}
          {waveformStyle === "minimal" && <div className="w-2" />}
          <span className="text-[12px] leading-[18px] font-semibold text-accent shrink-0">
            Listening
          </span>
        </div>
      ) : isProcessing ? (
        <div className={`${hudContainerClass} overlay-hud-pill w-fit mx-auto max-w-full h-[56px] flex items-center justify-center gap-3 px-5`}>
          <span className="spinner-ring spinner-ring-accent" aria-label="transcribing" />
          <span className="text-[12px] leading-[18px] font-semibold text-accent">
            {polishing ? "Polishing" : "Transcribing"}
          </span>
        </div>
      ) : isInjecting ? (
        <div className={`${hudContainerClass} overlay-hud-pill w-fit mx-auto max-w-full h-[56px] flex items-center justify-center gap-3 px-5`}>
          <span className="spinner-ring spinner-ring-accent" aria-label="inserting" />
          <span className={`text-[12px] leading-[18px] font-medium ${isLightHud ? "text-ink" : "text-[#f8fafc]"}`}>
            Inserting…
          </span>
        </div>
      ) : isError ? (
        <div className={`${hudContainerClass} overlay-hud-pill w-fit mx-auto max-w-full h-[56px] flex items-center justify-center gap-2.5 px-5`}>
          <AlertCircle className="w-4 h-4 text-rose-500 shrink-0" />
          <span className="text-[12px] leading-[18px] font-medium text-rose-500 truncate">
            {extraMessage?.trim() || "Error"}
          </span>
        </div>
      ) : (
        <div
          className={`${hudContainerClass} w-full flex flex-col justify-center px-5 ${
            showPreviewLine
              ? "overlay-hud-card min-h-[88px] h-full py-3 gap-1.5"
              : "overlay-hud-pill h-[56px]"
          }`}
        >
          <div className="flex items-center gap-2.5 min-w-0">
            <div
              className={`w-5 h-5 rounded-full flex items-center justify-center shrink-0 ${
                neutral
                  ? "bg-slate-500/30 text-muted"
                  : "bg-emerald-400/15 text-emerald-400"
              }`}
            >
              <Check className="w-3.5 h-3.5 stroke-[2.5]" />
            </div>
            <span
              className={`text-[12px] leading-[18px] font-medium truncate ${
                neutral ? "text-muted" : "text-emerald-400"
              }`}
            >
              {statusLabel}
            </span>
          </div>
          {showPreviewLine && (
            <p
              className={`text-[13px] leading-[20px] truncate pl-7 ${
                isLightHud ? "text-ink" : "text-[#f8fafc]"
              }`}
              title={previewText}
            >
              {previewText}
            </p>
          )}
        </div>
      )}
    </div>
  );
};
