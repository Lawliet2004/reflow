import React, { useEffect, useState } from "react";
import {
  Mic,
  Square,
  Copy,
  Check,
  Loader2,
  ArrowUpRight,
  ShieldCheck,
  RotateCcw,
} from "lucide-react";
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
  IntelligenceTierState,
} from "../types";
import { api } from "../services/tauriApi";
import { FileTranscription, FileTranscriptionController } from "./FileTranscription";
import { LanguageOptions } from "./LanguageOptions";
import { derivePhase, BackendStage } from "./hud/stages";
import { Waveform } from "./Waveform";
import { LatencyWaterfall } from "./LatencyWaterfall";
import { StageRail } from "./hud/StageRail";
import { relativeTime } from "../historyDisplay";
import { LlmSelector } from "./LlmSelector";
import { selectedLlm } from "../llmSelection";

interface DictateHomeProps {
  appState: AppState;
  settings: AppSettings;
  modelStatus: ModelStatus | null;
  transcript: StreamingTranscriptPayload;
  originalText?: string;
  asrProgress?: { completed: number; total: number } | null;
  latencyMetrics: LatencyMetrics | null;
  latencyPercentiles: LatencyPercentiles | null;
  onStartRecording: () => void;
  onStopRecording: () => void;
  onUpdateSettings: (settings: Partial<AppSettings>) => Promise<boolean> | void;
  intelligenceTiers?: IntelligenceTierState[] | null;
  onOpenHistory: () => void;
  onOpenSettings?: () => void;
  backendStage?: BackendStage | null;
  fileTranscription?: FileTranscriptionController;
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
  originalText,
  asrProgress,
  latencyMetrics,
  latencyPercentiles,
  onStartRecording,
  onStopRecording,
  onUpdateSettings,
  intelligenceTiers,
  onOpenHistory,
  onOpenSettings,
  backendStage = null,
  fileTranscription,
}) => {
  const [draft, setDraft] = useState<{ source: string; text: string } | null>(null);
  const [recent, setRecent] = useState<HistoryEntry[]>([]);
  const [feedback, setFeedback] = useState<{ error: boolean; text: string } | null>(null);
  const [copied, setCopied] = useState<string | null>(null);
  const [seconds, setSeconds] = useState(0);
  const isRecording = appState === "RECORDING";
  useEffect(() => {
    if (!isRecording) return;
    const cancel = (event: KeyboardEvent) => {
      if (event.key === "Escape" && settings.hotkeys?.cancel !== null) {
        void api
          .cancelRecording()
          .catch((error: unknown) => setFeedback({ error: true, text: String(error) }));
      }
    };
    window.addEventListener("keydown", cancel);
    return () => window.removeEventListener("keydown", cancel);
  }, [isRecording, settings.hotkeys?.cancel]);
  const [previousRecording, setPreviousRecording] = useState(isRecording);
  // Reset synchronously on a new session, including global-shortcut starts.
  // A deferred effect can be cancelled by a fast stop and retain old edits.
  if (previousRecording !== isRecording) {
    setPreviousRecording(isRecording);
    if (isRecording) {
      setDraft(null);
      setSeconds(0);
    }
  }
  const isProcessing = appState === "PROCESSING" || appState === "INJECTING";
  const modelReady = isModelReady(modelStatus);
  const canStart = modelReady && (appState === "READY" || appState === "IDLE");
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
    return () => {
      clearInterval(timer);
    };
  }, [isRecording]);
  useEffect(() => {
    if (!feedback) return;
    const timer = setTimeout(() => setFeedback(null), 5000);
    return () => clearTimeout(timer);
  }, [feedback]);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(null), 1500);
    return () => clearTimeout(timer);
  }, [copied]);
  // Success is confirmed on the button that was pressed; only failures get a message.
  const copy = async (text: string, key: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setFeedback(null);
      setCopied(key);
    } catch {
      setCopied(null);
      setFeedback({ error: true, text: "Could not copy. Select the text and copy it manually." });
    }
  };
  const selectCleanup = (id: CleanupLevel) =>
    onUpdateSettings({
      cleanup_level: id,
      ...(id === "raw"
        ? { intelligence_tier: "raw_verbatim" as const, flow_model: "none" as const }
        : {}),
    });
  // Radio-group keyboard model: arrows move the selection, Tab leaves the group.
  const onCleanupKey = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const step = { ArrowRight: 1, ArrowDown: 1, ArrowLeft: -1, ArrowUp: -1 }[event.key];
    if (!step) return;
    event.preventDefault();
    if (isRecording || isProcessing) return;
    const available = polishEnabled
      ? CLEANUP
      : CLEANUP.filter((item) => item.id === "raw" || item.id === "light");
    const index =
      (available.findIndex((item) => item.id === cleanup) + step + available.length) %
      available.length;
    selectCleanup(available[index].id);
    (
      event.currentTarget.children[CLEANUP.findIndex((item) => item.id === available[index].id)] as
        HTMLElement | undefined
    )?.focus();
  };
  const polishEnabled = selectedLlm(settings) !== "none";
  const status = isRecording
    ? "Listening to you"
    : isProcessing
      ? appState === "INJECTING"
        ? "Inserting your words"
        : phase === "polish"
          ? "A final polish"
          : asrProgress?.total
            ? `Transcribing segment ${Math.min(asrProgress.completed + 1, asrProgress.total)} of ${asrProgress.total}`
            : "Finding your words"
      : modelReady
        ? "Ready when you are"
        : modelStatus?.is_downloading
          ? `Downloading speech model · ${Math.round(modelStatus.download_progress_pct ?? 0)}%`
          : "Let’s get your voice ready";
  return (
    <div className="workspace-page dictate-page animate-fade-rise">
      <header className="page-heading">
        <h1>Speak freely.</h1>
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
            <LanguageOptions />
          </select>
        </div>
        <div className="studio-center">
          <button
            className={`record-button ${isRecording ? "is-recording" : ""}`}
            // RMS speech sits around 0.05-0.3; the same 3.4x gain as the waveform.
            style={{ "--level": Math.min(1, transcript.audio_level * 3.4) } as React.CSSProperties}
            disabled={isProcessing || (!isRecording && !canStart)}
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
          {isRecording && (
            <button
              type="button"
              className="btn btn-ghost"
              onClick={() => {
                void api
                  .cancelRecording()
                  .catch((error: unknown) => setFeedback({ error: true, text: String(error) }));
              }}
            >
              Cancel · Esc
            </button>
          )}
          {isRecording ? (
            <div className="flex items-center gap-3">
              <Waveform level={transcript.audio_level} active barCount={24} height={24} />
              <span className="text-sm text-muted tabular-nums">
                {Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, "0")}
              </span>
            </div>
          ) : isProcessing ? (
            <div className="stage-track" data-phase={phase}>
              <StageRail phase={phase} polishEnabled={polishEnabled} />
            </div>
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
      {settings.meeting_mode && (
        <section
          className="panel p-4 flex items-center justify-between gap-4"
          aria-label="Meeting recording"
        >
          <div>
            <h2 className="font-semibold">Capture a meeting</h2>
            <p className="text-sm text-muted">
              Record your mic and computer playback. Saved to History without pasting.
            </p>
          </div>
          <button
            className="btn btn-secondary shrink-0"
            disabled={!canStart}
            onClick={() => {
              void api
                .startMeeting()
                .catch((e: unknown) => setFeedback({ error: true, text: String(e) }));
            }}
          >
            Record meeting
          </button>
        </section>
      )}
      <section className="cleanup-strip" aria-label="Writing preferences">
        <div>
          <h2>Make it sound like you.</h2>
          <p>{CLEANUP.find((item) => item.id === cleanup)?.description}</p>
        </div>
        <div
          className="segmented-control"
          role="radiogroup"
          aria-label="Cleanup level"
          tabIndex={-1}
          onKeyDown={onCleanupKey}
        >
          {CLEANUP.map((item) => (
            <button
              key={item.id}
              role="radio"
              aria-checked={cleanup === item.id}
              tabIndex={cleanup === item.id ? 0 : -1}
              disabled={
                isRecording ||
                isProcessing ||
                (!polishEnabled && (item.id === "medium" || item.id === "high"))
              }
              onClick={() => selectCleanup(item.id)}
            >
              {item.label}
            </button>
          ))}
        </div>
        <LlmSelector
          settings={settings}
          onUpdateSettings={onUpdateSettings}
          tiers={intelligenceTiers}
          disabled={isRecording || isProcessing}
          onOpenSettings={onOpenSettings}
        />
        {polishEnabled && (
          <label className="flex items-center gap-2 text-sm text-muted">
            Writing style
            <select
              className="field"
              value={settings.style ?? "neutral"}
              disabled={isRecording || isProcessing}
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
          aria-keyshortcuts="Control+Enter"
          onKeyDown={(event) => {
            if ((event.ctrlKey || event.metaKey) && event.key === "Enter" && editableText.trim()) {
              event.preventDefault();
              void copy(editableText, "main");
            }
          }}
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
            {originalText !== undefined && originalText !== editableText && (
              <button
                className="btn btn-ghost"
                disabled={isRecording || isProcessing}
                onClick={() => setDraft({ source: transcript.full_text, text: originalText })}
              >
                Use original
              </button>
            )}
            {draft?.source === transcript.full_text && (
              <button
                className="btn btn-ghost"
                disabled={isRecording || isProcessing}
                onClick={() => setDraft(null)}
              >
                Undo transcript edit
              </button>
            )}
            <button
              className="btn btn-primary min-w-[92px]"
              disabled={!editableText.trim()}
              title="Copy (Ctrl+Enter)"
              aria-keyshortcuts="Control+Enter"
              onClick={() => copy(editableText, "main")}
            >
              {copied === "main" ? <Check size={14} /> : <Copy size={14} />}
              {copied === "main" ? "Copied" : "Copy"}
            </button>
          </div>
        </footer>
      </section>
      {feedback && (
        <p className="action-feedback text-danger" role="alert">
          {feedback.text}
        </p>
      )}
      <span className="sr-only" role="status">
        {copied ? "Copied to clipboard." : ""}
      </span>
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
                <time
                  dateTime={entry.created_at}
                  title={new Date(entry.created_at).toLocaleString()}
                >
                  {relativeTime(entry.created_at)}
                </time>
                <button
                  className="icon-btn"
                  aria-label="Copy recent transcript"
                  onClick={() => copy(entry.final_transcript, String(entry.id))}
                >
                  {copied === String(entry.id) ? (
                    <Check size={14} className="text-success" />
                  ) : (
                    <Copy size={14} />
                  )}
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
      <FileTranscription
        onOpenHistory={onOpenHistory}
        disabled={isRecording || isProcessing}
        controller={fileTranscription}
      />
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
