import React, { useEffect, useMemo, useRef, useState } from "react";
import {
  Mic,
  Square,
  Copy,
  Check,
  CornerDownLeft,
  Loader2,
  RotateCcw,
  Volume2,
  Activity,
} from "lucide-react";
import {
  AppState,
  AppSettings,
  CleanupLevel,
  HistoryEntry,
  LatencyMetrics,
  ModelStatus,
  StreamingTranscriptPayload,
  TranscriptStyle,
  isModelReady,
} from "../types";
import { api } from "../services/tauriApi";
import { Waveform } from "./Waveform";

interface DictateHomeProps {
  appState: AppState;
  settings: AppSettings;
  modelStatus: ModelStatus | null;
  transcript: StreamingTranscriptPayload;
  latencyMetrics: LatencyMetrics | null;
  onStartRecording: () => void;
  onStopRecording: () => void;
  onUpdateSettings: (settings: Partial<AppSettings>) => void;
  onOpenHistory: () => void;
}

const LANGS: { id: string; label: string }[] = [
  { id: "auto", label: "Auto" },
  { id: "en", label: "English" },
  { id: "hi", label: "Hindi" },
];

const CLEANUP: { id: CleanupLevel; label: string }[] = [
  { id: "raw", label: "Raw" },
  { id: "light", label: "Light" },
  { id: "medium", label: "Medium" },
  { id: "high", label: "High" },
];

const STYLES: { id: TranscriptStyle; label: string }[] = [
  { id: "faithful", label: "Faithful" },
  { id: "neutral", label: "Neutral" },
  { id: "decisive", label: "Decisive" },
  { id: "email", label: "Email" },
  { id: "chat", label: "Chat" },
];

function timeAgo(iso: string): string {
  const s = (Date.now() - new Date(iso).getTime()) / 1000;
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)} min ago`;
  if (s < 86400) return `${Math.floor(s / 3600)} h ago`;
  return `${Math.floor(s / 86400)} d ago`;
}

function formatDuration(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}:${String(s).padStart(2, "0")}`;
}

export const DictateHome: React.FC<DictateHomeProps> = ({
  appState,
  settings,
  modelStatus,
  transcript,
  latencyMetrics,
  onStartRecording,
  onStopRecording,
  onUpdateSettings,
  onOpenHistory,
}) => {
  const [copied, setCopied] = useState(false);
  const [injected, setInjected] = useState(false);
  const [editableText, setEditableText] = useState("");
  const [showEditor, setShowEditor] = useState(false);
  const [recent, setRecent] = useState<HistoryEntry[]>([]);
  const [micLevel, setMicLevel] = useState(0);
  const [micTesting, setMicTesting] = useState(false);
  const [recordingSeconds, setRecordingSeconds] = useState(0);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const isRecording = appState === "RECORDING";
  const isProcessing = appState === "PROCESSING";
  const modelReady = isModelReady(modelStatus);
  const downloading = Boolean(modelStatus?.is_downloading);
  const cleanup = settings.cleanup_level ?? "light";
  const showStyle = cleanup === "medium" || cleanup === "high";

  useEffect(() => {
    if (transcript.full_text) {
      setEditableText(transcript.full_text);
      setShowEditor(false);
    }
  }, [transcript.full_text]);

  useEffect(() => {
    api.getHistory(3, 0).then((rows) => setRecent(rows.slice(0, 3))).catch(() => {});
  }, [appState, isProcessing]);

  useEffect(() => {
    if (isRecording) {
      setRecordingSeconds(0);
      timerRef.current = setInterval(() => setRecordingSeconds((s) => s + 1), 1000);
    } else {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
    }
    return () => {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [isRecording]);

  const hotkeyParts = useMemo(() => settings.hotkey.split("+"), [settings.hotkey]);

  const handleCopy = () => {
    if (!editableText) return;
    navigator.clipboard.writeText(editableText);
    setCopied(true);
    setTimeout(() => setCopied(false), 1600);
  };

  const handleInject = async () => {
    if (!editableText) return;
    try {
      await api.injectText(editableText);
      setInjected(true);
      setTimeout(() => setInjected(false), 1600);
    } catch (e) {
      console.error("Injection failed:", e);
    }
  };

  const orbDisabled = isProcessing || !modelReady;
  const polishing = (transcript.stage ?? "").toLowerCase() === "polishing";

  const startMicTest = async () => {
    setMicTesting(true);
    const tick = async () => {
      try {
        const lvl = await api.testMicrophoneLevel();
        setMicLevel(lvl);
      } catch {
        setMicLevel(0);
      }
    };
    await tick();
    const id = setInterval(tick, 100);
    setTimeout(() => {
      clearInterval(id);
      setMicTesting(false);
      setMicLevel(0);
    }, 3000);
  };

  return (
    <div className="max-w-2xl mx-auto px-7 py-8 animate-fade-rise">
      <header className="flex items-start justify-between mb-5">
        <div>
          <h1 className="font-display text-[22px] font-semibold tracking-tight text-ink">
            Dictate
          </h1>
          <p className="text-[12.5px] text-muted mt-0.5">
            Hold your hotkey in any app. This page is for a quick test.
          </p>
        </div>
        <select
          className="field !py-1.5 !text-[12.5px] !w-[120px]"
          value={settings.language}
          onChange={(e) => onUpdateSettings({ language: e.target.value })}
          aria-label="Language"
        >
          {LANGS.map((l) => (
            <option key={l.id} value={l.id}>
              {l.label}
            </option>
          ))}
        </select>
      </header>

      <section className="panel p-4 mb-5">
        <div className="flex items-center justify-between mb-2.5">
          <span className="text-[13px] font-semibold text-ink">Cleanup</span>
          <span className="text-[11.5px] text-muted">
            {cleanup === "raw"
              ? "ASR text as-is"
              : cleanup === "light"
                ? "Punctuation & fillers"
                : "Flow rewrite on-device"}
          </span>
        </div>
        <div className="flex rounded-xl border border-line overflow-hidden bg-base-2">
          {CLEANUP.map((item) => {
            const active = cleanup === item.id;
            return (
              <button
                key={item.id}
                onClick={() => onUpdateSettings({ cleanup_level: item.id })}
                className={`flex-1 py-2 text-[12.5px] font-semibold transition-colors cursor-pointer ${
                  active
                    ? "bg-surface text-accent shadow-xs"
                    : "text-muted hover:text-ink"
                }`}
              >
                {item.label}
              </button>
            );
          })}
        </div>
        {showStyle && (
          <div className="flex items-center justify-between mt-3">
            <span className="text-[12.5px] text-muted">Style</span>
            <select
              className="field !py-1.5 !text-[12.5px]"
              value={settings.style ?? "neutral"}
              onChange={(e) => onUpdateSettings({ style: e.target.value as TranscriptStyle })}
            >
              {STYLES.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.label}
                </option>
              ))}
            </select>
          </div>
        )}
      </section>

      <section className="panel p-4 mb-5">
        <div className="flex items-center gap-3">
          <button
            onClick={isRecording ? onStopRecording : onStartRecording}
            disabled={orbDisabled}
            title={modelReady ? (isRecording ? "Stop" : "Test dictation") : "Model not ready"}
            aria-label={isRecording ? "Stop recording" : "Start recording"}
            className={`w-11 h-11 rounded-full flex items-center justify-center shrink-0 transition-all cursor-pointer ${
              isRecording
                ? "bg-accent text-white shadow-[0_6px_18px_var(--color-accent)]"
                : isProcessing
                  ? "bg-accent-soft text-accent border border-accent-border"
                  : modelReady
                    ? "bg-accent-soft text-accent border border-accent-border hover:border-accent"
                    : "bg-surface-3 text-faint border border-line"
            }`}
          >
            {isProcessing ? (
              <Loader2 className="w-5 h-5 animate-spin" />
            ) : isRecording ? (
              <Square className="w-4 h-4 fill-current" />
            ) : (
              <Mic className="w-5 h-5" strokeWidth={1.9} />
            )}
          </button>

          <div className="flex-1 min-w-0">
            {isRecording ? (
              <div className="flex items-center gap-2">
                <Waveform
                  level={transcript.audio_level}
                  active
                  barCount={20}
                  height={22}
                />
                <span className="text-[13px] font-medium text-accent shrink-0">Listening</span>
                <span className="text-[12px] text-muted font-mono shrink-0 tabular-nums">
                  {formatDuration(recordingSeconds)}
                </span>
              </div>
            ) : isProcessing ? (
              <p className="text-[13px] font-medium text-accent">
                {polishing ? "Polishing…" : "Transcribing…"}
              </p>
            ) : modelReady ? (
              <div className="flex flex-wrap items-center gap-1.5 text-[13px] text-muted">
                <span>Hold</span>
                {hotkeyParts.map((p, i) => (
                  <React.Fragment key={i}>
                    {i > 0 && <span className="text-muted text-[11px]">+</span>}
                    <span className="kbd !text-[11px] !py-0.5 !px-1.5">{p}</span>
                  </React.Fragment>
                ))}
                <span>to speak</span>
              </div>
            ) : downloading ? (
              <p className="text-[13px] text-accent font-medium">
                Downloading model · {modelStatus?.download_progress_pct ?? 0}%
              </p>
            ) : (
              <p className="text-[13px] text-muted">Speech model isn't ready yet.</p>
            )}
          </div>

          <button
            onClick={startMicTest}
            disabled={micTesting}
            title="Test microphone for 3 seconds"
            aria-label="Test microphone"
            className="icon-btn border border-line hover:!bg-accent-soft hover:!text-accent shrink-0"
          >
            <Volume2 className="w-4 h-4" />
          </button>
        </div>
        {micTesting && (
          <div className="mt-3 flex items-center gap-2.5 text-[12px] text-muted">
            <Activity className="w-3.5 h-3.5 text-accent" />
            <div className="flex-1 h-1.5 rounded-full bg-line overflow-hidden">
              <div
                className="h-full bg-accent transition-all"
                style={{ width: `${Math.min(100, micLevel * 100 * 3)}%` }}
              />
            </div>
            <span className="font-mono text-[11px] w-10 text-right tabular-nums">
              {(micLevel * 100).toFixed(0)}
            </span>
          </div>
        )}
      </section>

      {settings.developer_mode && latencyMetrics && latencyMetrics.speech_end_to_final_ms > 0 && (
        <div className="flex items-center gap-2 mb-5">
          <span className="chip" title="Speech end to final">
            {Math.round(latencyMetrics.speech_end_to_final_ms)} ms
            {typeof latencyMetrics.rewrite_ms === "number"
              ? ` · rewrite ${Math.round(latencyMetrics.rewrite_ms)} ms`
              : ""}
          </span>
        </div>
      )}

      {!isRecording && (
        <section className="panel p-4 mb-5">
          <div className="flex items-center justify-between mb-2">
            <span className="text-[13px] font-semibold text-ink">Last transcript</span>
            {editableText.trim() ? (
              <span className="text-[11px] text-muted tabular-nums">
                {editableText.trim().split(/\s+/).length} words · {editableText.length} chars
              </span>
            ) : (
              <span className="text-[11px] text-muted">Nothing yet</span>
            )}
          </div>

          {editableText.trim() && !showEditor ? (
            <p
              className="text-[14px] leading-6 text-ink whitespace-pre-wrap break-words"
              title={editableText}
            >
              {editableText}
            </p>
          ) : (
            <textarea
              value={editableText}
              onChange={(e) => setEditableText(e.target.value)}
              placeholder="Your last transcript lands here."
              className="w-full min-h-[72px] bg-transparent text-[14px] leading-6 text-ink placeholder:text-muted focus:outline-none resize-none"
            />
          )}

          <div className="flex items-center justify-between pt-2.5 mt-1 border-t border-line">
            <div>
              {editableText && (
                <button
                  onClick={() => setEditableText("")}
                  className="icon-btn text-muted hover:text-ink hover:bg-base-2"
                  title="Clear text"
                  aria-label="Clear text"
                >
                  <RotateCcw className="w-3.5 h-3.5" />
                </button>
              )}
            </div>
            <div className="flex items-center gap-2">
              {editableText && !showEditor && (
                <button
                  onClick={() => setShowEditor(true)}
                  className="btn btn-ghost !py-1.5 !px-3 !text-[12.5px]"
                >
                  Edit
                </button>
              )}
              <button
                onClick={handleCopy}
                disabled={!editableText}
                className="btn btn-ghost !py-1.5 !px-3 !text-[12.5px] disabled:opacity-40"
              >
                {copied ? (
                  <>
                    <Check className="w-3.5 h-3.5 text-emerald-600 dark:text-emerald-400" />
                    <span className="text-emerald-600 dark:text-emerald-400">Copied</span>
                  </>
                ) : (
                  <>
                    <Copy className="w-3.5 h-3.5 text-muted" />
                    <span>Copy</span>
                  </>
                )}
              </button>
              <button
                onClick={handleInject}
                disabled={!editableText}
                className="btn btn-primary !py-1.5 !px-4 !text-[12.5px] disabled:opacity-40"
              >
                {injected ? (
                  <>
                    <Check className="w-3.5 h-3.5 stroke-[2.5]" />
                    <span>Inserted</span>
                  </>
                ) : (
                  <>
                    <CornerDownLeft className="w-3.5 h-3.5" />
                    <span>Insert</span>
                  </>
                )}
              </button>
            </div>
          </div>
        </section>
      )}

      {!isRecording && recent.length > 0 && (
        <section>
          <div className="flex items-center justify-between mb-3 px-1">
            <span className="text-[13px] font-semibold text-ink">Recent</span>
            <button
              onClick={onOpenHistory}
              className="flex items-center gap-1 text-[12px] font-semibold text-accent hover:text-accent-hover transition-colors cursor-pointer"
            >
              View all →
            </button>
          </div>
          <div className="panel divide-y divide-line overflow-hidden">
            {recent.map((entry) => (
              <div
                key={entry.id}
                className="flex items-center gap-3 px-4 py-3 hover:bg-base-2 transition-colors"
              >
                <p
                  className="flex-1 min-w-0 truncate text-[13px] text-ink-2"
                  title={entry.final_transcript}
                >
                  {entry.final_transcript}
                </p>
                <span className="text-[11px] text-muted whitespace-nowrap">
                  {timeAgo(entry.created_at)}
                </span>
                <button
                  className="icon-btn hover:bg-base-2"
                  title="Copy transcript"
                  aria-label="Copy transcript"
                  onClick={() => navigator.clipboard.writeText(entry.final_transcript)}
                >
                  <Copy className="w-3.5 h-3.5" />
                </button>
              </div>
            ))}
          </div>
        </section>
      )}

      <p className="text-[11.5px] text-muted text-center mt-8">
        Close hides to tray · Quit from the sidebar or tray menu
      </p>
    </div>
  );
};
