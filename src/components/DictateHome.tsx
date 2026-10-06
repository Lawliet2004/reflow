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
import { createEventScope } from "../services/eventScope";
import { FileTranscription, FileTranscriptionController } from "./FileTranscription";
import { LanguageOptions } from "./LanguageOptions";
import { derivePhase, BackendStage } from "./hud/stages";
import { Waveform } from "./Waveform";
import { LatencyWaterfall } from "./LatencyWaterfall";
import { StageRail } from "./hud/StageRail";
import { relativeTime } from "../historyDisplay";
import { LlmSelector } from "./LlmSelector";
import { selectedLlm } from "../llmSelection";
import { WritingTaskPicker } from "./WritingTaskPicker";

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
  {
    id: "light",
    label: "Minimal",
    description: "Tidy punctuation and fillers; make minimal changes.",
  },
  {
    id: "medium",
    label: "Natural",
    description: "Repair grammar and repetitions while keeping your voice.",
  },
  {
    id: "high",
    label: "Structured",
    description: "Improve flow and organize longer text into paragraphs.",
  },
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
  const [historyRevision, setHistoryRevision] = useState(0);
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
    const scope = createEventScope();
    void scope.listen<HistoryEntry>("history:updated", () => {
      setHistoryRevision((value) => value + 1);
    });
    return () => scope.dispose();
  }, []);
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
  }, [isRecording, isProcessing, historyRevision]);
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
  const words = editableText.trim() ? editableText.trim().split(/\s+/).length : 0;
  const busy = isRecording || isProcessing;
  return (
    <div className="workspace-page dictate-page animate-fade-rise">
      <header className="page-heading">
        <h1>Speak freely.</h1>
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
      </header>
      <section className="manuscript" aria-label="Dictation studio">
        <div className="manuscript-head">
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
              <Loader2 size={26} className="animate-spin" />
            ) : isRecording ? (
              <Square size={22} fill="currentColor" />
            ) : (
              <Mic size={28} strokeWidth={1.5} />
            )}
          </button>
          <div className="manuscript-status">
            <h2>{status}</h2>
            {isRecording ? (
              <div className="manuscript-live">
                <Waveform level={transcript.audio_level} active barCount={24} height={22} />
                <span className="text-sm text-muted tabular-nums">
                  {Math.floor(seconds / 60)}:{String(seconds % 60).padStart(2, "0")}
                </span>
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
            ) : modelStatus?.is_downloading ? (
              <p>You can keep working while it downloads.</p>
            ) : (
              <>
                <p>Download a speech model once. After that, everything runs on this computer.</p>
                {onOpenSettings && (
                  <button className="btn btn-primary" onClick={onOpenSettings}>
                    Set up speech model <ArrowUpRight size={14} />
                  </button>
                )}
              </>
            )}
          </div>
        </div>
        <textarea
          aria-label="Transcript"
          className="manuscript-text"
          value={editableText}
          readOnly={busy}
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
        <footer className="manuscript-foot">
          <span>
            <ShieldCheck size={14} aria-hidden /> Speech is processed on your computer.
            {words > 0 && ` · ${words} ${words === 1 ? "word" : "words"}`}
          </span>
          <div>
            <button
              className="icon-btn"
              aria-label="Clear transcript"
              title="Clear transcript"
              disabled={!editableText || busy}
              onClick={() => setDraft({ source: transcript.full_text, text: "" })}
            >
              <RotateCcw size={15} />
            </button>
            {originalText !== undefined && originalText !== editableText && (
              <button
                className="btn btn-ghost"
                disabled={busy}
                onClick={() => setDraft({ source: transcript.full_text, text: originalText })}
              >
                Use original
              </button>
            )}
            {draft?.source === transcript.full_text && (
              <button className="btn btn-ghost" disabled={busy} onClick={() => setDraft(null)}>
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
      <section className="writing-bar" aria-label="Writing preferences">
        <WritingTaskPicker
          settings={settings}
          onUpdateSettings={onUpdateSettings}
          disabled={busy}
        />
        <div className="writing-bar-intro">
          <h2>Fine-tune the edit.</h2>
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
              disabled={busy || (!polishEnabled && (item.id === "medium" || item.id === "high"))}
              onClick={() => selectCleanup(item.id)}
            >
              {item.label}
            </button>
          ))}
        </div>
        <div className="writing-bar-models">
          <LlmSelector
            settings={settings}
            onUpdateSettings={onUpdateSettings}
            tiers={intelligenceTiers}
            disabled={busy}
            onOpenSettings={onOpenSettings}
          />
          {polishEnabled && (
            <label>
              Writing style
              <select
                className="field"
                value={settings.style ?? "neutral"}
                disabled={busy}
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
        </div>
      </section>
      {settings.meeting_mode && (
        <section
          className="panel p-5 mt-8 flex items-center justify-between gap-4"
          aria-label="Meeting recording"
        >
          <div>
            <h2 className="font-display text-lg font-medium">Capture a meeting</h2>
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
      <div className="home-secondary">
        <section className="recent-section" aria-label="Recently said">
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
            <p className="recent-empty">
              Your finished dictations will be saved here. One less thought to lose.
            </p>
          )}
        </section>
        <FileTranscription
          onOpenHistory={onOpenHistory}
          disabled={busy}
          controller={fileTranscription}
        />
      </div>
      {settings.developer_mode && latencyMetrics && (
        <details className="panel p-5 mt-8">
          <summary className="cursor-pointer text-sm font-medium">Dictation diagnostics</summary>
          <div className="mt-4">
            <LatencyWaterfall metrics={latencyMetrics} percentiles={latencyPercentiles} />
          </div>
        </details>
      )}
    </div>
  );
};
