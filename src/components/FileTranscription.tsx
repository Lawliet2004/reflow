import React, { useCallback, useEffect, useId, useRef, useState } from "react";
import { Check, FileAudio, FolderOpen, Loader2 } from "lucide-react";
import { api, isTauri } from "../services/tauriApi";
import { FileProgress } from "../types";

export function useFileTranscriptionController(disabled = false) {
  const [path, setPath] = useState("");
  const [allowLarge, setAllowLarge] = useState(false);
  const [job, setJob] = useState<FileProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const [cancelRequested, setCancelRequested] = useState(false);
  const startPending = useRef(false);
  const jobId = job?.job_id;
  const finished = job?.finished;
  const running = Boolean(job && !job.finished);
  const locked = running || busy || choosing || Boolean(disabled);
  const cancelled = job?.finished && job.error === "File transcription cancelled";
  const failure = error || (!cancelled ? job?.error : null);
  const selectPath = useCallback((value: string) => {
    setPath(value);
    setJob(null);
    setError(null);
    setCancelRequested(false);
  }, []);
  useEffect(() => {
    if (!jobId || finished) return;
    let alive = true;
    let pending = false;
    const poll = async () => {
      if (pending) return;
      pending = true;
      try {
        const result = await api.getFileJob(jobId);
        if (alive) setJob(result);
      } catch (e) {
        if (alive) setError(String(e));
      } finally {
        pending = false;
      }
    };
    void poll();
    const timer = setInterval(poll, 700);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [jobId, finished]);
  const start = async () => {
    if (!path.trim() || locked || startPending.current) return;
    startPending.current = true;
    setBusy(true);
    setError(null);
    setJob(null);
    setCancelRequested(false);
    try {
      const result = await api.transcribeFile(path.trim(), allowLarge);
      setJob({ ...result, done_s: 0, total_s: 0, finished: false, error: null, history_id: null });
    } catch (e) {
      setError(String(e));
    } finally {
      startPending.current = false;
      setBusy(false);
    }
  };
  const choose = async () => {
    if (locked) return;
    setChoosing(true);
    setError(null);
    try {
      const value = await api.selectAudioFile();
      if (value) selectPath(value);
    } catch (e) {
      setError(String(e));
    } finally {
      setChoosing(false);
    }
  };
  const cancel = async () => {
    if (!job || job.finished || cancelRequested) return;
    setCancelRequested(true);
    try {
      await api.cancelFileTranscription(job.job_id);
    } catch (e) {
      setCancelRequested(false);
      setError(String(e));
    }
  };
  return {
    path,
    allowLarge,
    setAllowLarge,
    job,
    busy,
    choosing,
    cancelRequested,
    failure,
    cancelled,
    selectPath,
    setError,
    start,
    choose,
    cancel,
  };
}

export type FileTranscriptionController = ReturnType<typeof useFileTranscriptionController>;
interface FileTranscriptionProps {
  onOpenHistory: () => void;
  disabled?: boolean;
  controller?: FileTranscriptionController;
}

export const FileTranscription: React.FC<FileTranscriptionProps> = (props) =>
  props.controller ? (
    <FileTranscriptionView {...props} controller={props.controller} />
  ) : (
    <LocalFileTranscription {...props} />
  );

function LocalFileTranscription(props: FileTranscriptionProps) {
  const controller = useFileTranscriptionController(props.disabled);
  return <FileTranscriptionView {...props} controller={controller} />;
}

function FileTranscriptionView({
  onOpenHistory,
  disabled,
  controller,
}: FileTranscriptionProps & { controller: FileTranscriptionController }) {
  const {
    path,
    allowLarge,
    setAllowLarge,
    job,
    busy,
    choosing,
    cancelRequested,
    failure,
    cancelled,
    selectPath,
    setError,
    start,
    choose,
    cancel,
  } = controller;
  const pathId = useId();
  const running = Boolean(job && !job.finished);
  const locked = running || busy || choosing || Boolean(disabled);
  const filename = path.trim().split(/[\\/]/).pop();
  const preparing = running && !job?.total_s;
  useEffect(() => {
    if (!isTauri()) return;
    let alive = true;
    let dispose: (() => void) | undefined;
    void import("@tauri-apps/api/webviewWindow")
      .then(async ({ getCurrentWebviewWindow }) => {
        const unlisten = await getCurrentWebviewWindow().onDragDropEvent((event) => {
          if (alive && event.payload.type === "drop" && !locked)
            selectPath(event.payload.paths[0] ?? "");
        });
        if (alive) dispose = unlisten;
        else unlisten();
      })
      .catch((e) => {
        if (alive) setError(String(e));
      });
    return () => {
      alive = false;
      dispose?.();
    };
  }, [locked, selectPath, setError]);
  return (
    <section className="panel p-5 space-y-4" aria-label="File transcription">
      <div>
        <h2 className="font-semibold text-sm">Transcribe an audio file</h2>
        <p className="text-xs text-muted mt-1 leading-relaxed">
          Browse or drop a WAV, MP3, M4A, AAC, FLAC or Ogg file. Up to two hours, processed on your
          computer.
        </p>
      </div>
      <div className="space-y-2">
        <label htmlFor={pathId} className="block text-xs font-medium text-ink-2">
          Audio file path
        </label>
        <div className="flex flex-wrap gap-2">
          <input
            id={pathId}
            className="field flex-1 min-w-0 basis-48"
            placeholder="Choose a file or paste its path"
            title={path || undefined}
            value={path}
            disabled={locked}
            onChange={(e) => selectPath(e.target.value)}
          />
          <button className="btn btn-secondary shrink-0" disabled={locked} onClick={choose}>
            {choosing ? <Loader2 size={14} className="animate-spin" /> : <FolderOpen size={14} />}
            {choosing ? "Choosing…" : "Browse"}
          </button>
        </div>
        {filename && (
          <p className="flex items-center gap-2 text-xs text-muted min-w-0" title={path.trim()}>
            <FileAudio size={14} className="shrink-0" aria-hidden />
            <span className="truncate">{filename}</span>
          </p>
        )}
      </div>
      <label className="flex items-start gap-2 text-xs text-muted leading-relaxed">
        <input
          type="checkbox"
          className="mt-0.5 shrink-0 accent-accent"
          checked={allowLarge}
          disabled={locked}
          onChange={(e) => setAllowLarge(e.target.checked)}
        />
        Allow files larger than 100 MB (uses more memory)
      </label>
      {running ? (
        <div role="status" className="space-y-2">
          <progress
            className="w-full h-1.5 accent-accent"
            aria-label="File transcription progress"
            max={job?.total_s || 1}
            value={preparing ? undefined : Math.min(job?.done_s ?? 0, job?.total_s ?? 0)}
          />
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="text-xs text-muted tabular-nums">
              {cancelRequested
                ? "Stopping the import…"
                : preparing
                  ? "Preparing audio…"
                  : `${Math.round(job?.done_s ?? 0)} of ${Math.round(job?.total_s ?? 0)} seconds transcribed`}
            </span>
            <button className="btn btn-ghost" disabled={cancelRequested} onClick={cancel}>
              {cancelRequested ? "Cancelling…" : "Cancel import"}
            </button>
          </div>
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-3">
          <button className="btn btn-secondary" disabled={!path.trim() || locked} onClick={start}>
            {busy && <Loader2 size={14} className="animate-spin" aria-hidden />}
            {busy ? "Starting…" : "Transcribe file"}
          </button>
          {disabled && !busy && (
            <p className="text-xs text-muted">
              Finish your current recording before importing a file.
            </p>
          )}
        </div>
      )}
      {job?.finished && job.history_id && (
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p role="status" className="flex items-center gap-2 text-xs text-muted">
            <Check size={14} className="text-success" aria-hidden /> Transcript saved to History.
          </p>
          <button className="btn btn-ghost" onClick={onOpenHistory}>
            Open completed transcript in History
          </button>
        </div>
      )}
      {cancelled && (
        <p role="status" className="text-xs text-muted">
          Import cancelled.
        </p>
      )}
      {failure && (
        <p role="alert" className="text-sm text-danger break-words">
          {failure}
        </p>
      )}
    </section>
  );
}
