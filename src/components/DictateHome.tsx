import React, { useEffect, useState } from "react";
import { Mic, Square, Copy, Loader2, ArrowUpRight, ShieldCheck, RotateCcw } from "lucide-react";
import {
  AppState,
  AppSettings,
  CleanupLevel,
  HistoryEntry,
  LatencyMetrics,
  LatencyPercentiles,
  ModelStatus,
  StreamingTranscriptPayload,
  TranscriptStyle,
  isModelReady,
} from "../types";
import { api } from "../services/tauriApi";
import { derivePhase, BackendStage } from "./hud/stages";
import { Waveform } from "./Waveform";
import { LatencyWaterfall } from "./LatencyWaterfall";

interface DictateHomeProps {
  appState: AppState;
  settings: AppSettings;
  modelStatus: ModelStatus | null;
  transcript: StreamingTranscriptPayload;
  latencyMetrics: LatencyMetrics | null;
  latencyPercentiles: LatencyPercentiles | null;
  onStartRecording: () => void;
  onStopRecording: () => void;
  onUpdateSettings: (settings: Partial<AppSettings>) => void;
  onOpenHistory: () => void;
  onOpenSettings?: () => void;
  backendStage?: BackendStage | null;
}
const CLEANUP: { id: CleanupLevel; label: string; description: string }[] = [
  { id: "raw", label: "Original", description: "Keep the words exactly as recognized." },
  { id: "light", label: "Clean", description: "Tidy punctuation and remove filler words." },
  { id: "medium", label: "Polished", description: "Smooth the phrasing with on-device AI." },
  { id: "high", label: "Refined", description: "Give your words a more considered edit." },
];
export const DictateHome: React.FC<DictateHomeProps> = ({
  appState,
  settings,
  modelStatus,
  transcript,
  latencyMetrics,
  latencyPercentiles,
  onStartRecording,
  onStopRecording,
  onUpdateSettings,
  onOpenHistory,
  onOpenSettings,
  backendStage = null,
}) => {
  const [draft, setDraft] = useState<{ source: string; text: string } | null>(null);
  const [recent, setRecent] = useState<HistoryEntry[]>([]);
  const [feedback, setFeedback] = useState<{ error: boolean; text: string } | null>(null);
  const [seconds, setSeconds] = useState(0);
  const isRecording = appState === "RECORDING";
  const isProcessing = appState === "PROCESSING";
  const modelReady = isModelReady(modelStatus);
  const editableText = draft?.source === transcript.full_text ? draft.text : transcript.full_text;
  const cleanup = settings.cleanup_level ?? "light";
  const phase = derivePhase(appState, transcript, null, backendStage);
  useEffect(() => {
    let alive = true;
    if (!isRecording && !isProcessing)
      api
        .getHistory(3, 0)
        .then((rows) => {
          if (alive) setRecent(rows);
        })
        .catch(() => {});
    return () => {
      alive = false;
    };
  }, [isRecording, isProcessing]);
  useEffect(() => {
    if (!isRecording) return;
    const started = Date.now();
    const timer = setInterval(() => setSeconds(Math.floor((Date.now() - started) / 1000)), 1000);
    return () => clearInterval(timer);
  }, [isRecording]);
  useEffect(() => {
    if (!feedback) return;
    const timer = setTimeout(() => setFeedback(null), 5000);
    return () => clearTimeout(timer);
  }, [feedback]);
  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setFeedback({ error: false, text: "Copied to clipboard." });
    } catch {
      setFeedback({ error: true, text: "Could not copy. Select the text and copy it manually." });
    }
  };
  const status = isRecording
    ? "Listening to you"
    : isProcessing
      ? phase === "polish"
        ? "A final polish"
        : "Finding your words"
      : modelReady
        ? "Ready when you are"
        : modelStatus?.is_downloading
          ? `Downloading speech model · ${Math.round(modelStatus.download_progress_pct ?? 0)}%`
          : "Let’s get your voice ready";
  return (
    <div className="workspace-page dictate-page animate-fade-rise">
      <header className="page-heading">
        <div>
          <p className="eyebrow">YOUR PERSONAL DICTATION SPACE</p>
          <h1>
            Speak freely.
            <br />
            <span className="text-muted">Let your words flow.</span>
          </h1>
        </div>
        <span className="privacy-label">
          <ShieldCheck size={15} /> On-device by design
        </span>
      </header>
      <section className="dictation-studio" aria-label="Dictation studio">
        <div className="studio-toolbar">
          <span className="flex items-center gap-2 text-sm font-medium">
            <span className={`status-dot ${isRecording ? "is-live" : ""}`} />
            {isRecording ? "Recording" : isProcessing ? "Processing" : "Dictation"}
          </span>
          <select
            className="field language-picker"
            aria-label="Language"
            value={settings.language}
            onChange={(event) =>
              onUpdateSettings({
                language: event.target.value,
                auto_detect_language: event.target.value === "auto",
              })
            }
          >
            <option value="auto">Auto-detect</option>
            <option value="en">English</option>
            <option value="hi">Hindi</option>
          </select>
        </div>
        <div className="studio-center">
          <button
            className={`record-button ${isRecording ? "is-recording" : ""}`}
            disabled={isProcessing || (!isRecording && !modelReady)}
            aria-label={isRecording ? "Stop recording" : "Start recording"}
            onClick={() => {
              if (isRecording) onStopRecording();
              else {
                setSeconds(0);
                onStartRecording();
              }
            }}
          >
            {isProcessing ? (
              <Loader2 size={30} className="animate-spin" />
            ) : isRecording ? (
              <Square size={26} fill="currentColor" />
            ) : (
              <Mic size={32} strokeWidth={1.5} />
            )}
          </button>
          <h2>{status}</h2>
          {isRecording ? (
            <div className="flex items-center gap-3">
              <Waveform level={transcript.audio_level} active barCount={24} height={24} />
              <span className="text-sm text-muted tabular-nums">
                {Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, "0")}
              </span>
            </div>
          ) : isProcessing ? (
            <p>Your transcript will appear below.</p>
          ) : modelReady ? (
            <p className="shortcut-hint">
              {!settings.push_to_talk ? "Press" : "Hold"}{" "}
              {settings.hotkey.split("+").map((key, index) => (
                <kbd key={index} className="kbd">
                  {key}
                </kbd>
              ))}{" "}
              in any app, or click the microphone.
            </p>
          ) : (
            <p>
              Set up your speech model in{" "}
              <button className="text-link" onClick={onOpenSettings}>
                Settings <ArrowUpRight size={13} />
              </button>
            </p>
          )}
          {isRecording && transcript.full_text && (
            <p className="live-transcript">{transcript.full_text}</p>
          )}
        </div>
        <div className="studio-footer">
          <span>
            <ShieldCheck size={14} /> Speech is processed on your computer.
          </span>
        </div>
      </section>
      <section className="cleanup-strip" aria-label="Writing preferences">
        <div>
          <h2>Make it sound like you.</h2>
          <p>{CLEANUP.find((item) => item.id === cleanup)?.description}</p>
        </div>
        <div className="segmented-control" aria-label="Cleanup level">
          {CLEANUP.map((item) => (
            <button
              key={item.id}
              aria-pressed={cleanup === item.id}
              onClick={() =>
                onUpdateSettings({
                  cleanup_level: item.id,
                  intelligence_tier:
                    item.id === "raw" || item.id === "light"
                      ? "raw_verbatim"
                      : settings.intelligence_tier === "deep_context"
                        ? "deep_context"
                        : "smart_flow",
                })
              }
            >
              {item.label}
            </button>
          ))}
        </div>
        {(cleanup === "medium" || cleanup === "high") && (
          <label className="flex items-center gap-2 text-sm text-muted">
            Writing style
            <select
              className="field"
              value={settings.style ?? "neutral"}
              onChange={(event) =>
                onUpdateSettings({ style: event.target.value as TranscriptStyle })
              }
            >
              {["faithful", "neutral", "decisive", "email", "chat"].map((style) => (
                <option value={style} key={style}>
                  {style[0].toUpperCase() + style.slice(1)}
                </option>
              ))}
            </select>
          </label>
        )}
      </section>
      <section className="transcript-sheet" aria-label="Your transcript">
        <header>
          <h2>Your words</h2>
          <span>
            {editableText.trim()
              ? `${editableText.trim().split(/\s+/).length} words`
              : "A clear space for your next thought"}
          </span>
        </header>
        <textarea
          aria-label="Transcript"
          value={editableText}
          readOnly={isRecording || isProcessing}
          onChange={(event) => setDraft({ source: transcript.full_text, text: event.target.value })}
          placeholder="Start speaking. Your transcript will appear here, ready to edit and copy into any app."
        />
        <footer>
          <button
            className="icon-btn"
            aria-label="Clear transcript"
            disabled={!editableText || isRecording || isProcessing}
            onClick={() => setDraft({ source: transcript.full_text, text: "" })}
          >
            <RotateCcw size={15} />
          </button>
          <div className="flex items-center gap-2">
            <button
              className="btn btn-primary"
              disabled={!editableText.trim()}
              onClick={() => copy(editableText)}
            >
              <Copy size={14} />
              Copy
            </button>
          </div>
        </footer>
      </section>
      {feedback && (
        <p
          className={`action-feedback ${feedback.error ? "text-danger" : "text-muted"}`}
          role={feedback.error ? "alert" : "status"}
        >
          {feedback.text}
        </p>
      )}
      <section className="recent-section">
        <header>
          <h2>Recently said</h2>
          <button className="text-link" onClick={onOpenHistory}>
            View history <ArrowUpRight size={14} />
          </button>
        </header>
        {recent.length ? (
          <div className="recent-list">
            {recent.map((entry) => (
              <div key={entry.id}>
                <p>{entry.final_transcript}</p>
                <time dateTime={entry.created_at}>
                  {new Date(entry.created_at).toLocaleDateString(undefined, {
                    month: "short",
                    day: "numeric",
                  })}
                </time>
                <button
                  className="icon-btn"
                  aria-label="Copy recent transcript"
                  onClick={() => copy(entry.final_transcript)}
                >
                  <Copy size={14} />
                </button>
              </div>
            ))}
          </div>
        ) : (
          <p className="text-sm text-muted py-5">
            Your finished dictations will be saved here. One less thought to lose.
          </p>
        )}
      </section>
      {settings.developer_mode && latencyMetrics && (
        <details className="panel p-5">
          <summary className="cursor-pointer text-sm font-medium">Dictation diagnostics</summary>
          <div className="mt-4">
            <LatencyWaterfall metrics={latencyMetrics} percentiles={latencyPercentiles} />
          </div>
        </details>
      )}
    </div>
  );
};
