import React, { useEffect, useRef, useState } from "react";
import {
  ArrowDownToLine,
  Check,
  ChevronDown,
  ChevronLeft,
  ChevronRight,
  FolderOpen,
  Loader2,
  Mic,
  NotebookPen,
  Search,
  Square,
  X,
} from "lucide-react";
import { api } from "../services/tauriApi";
import { AppSettings, HistoryEntry, AppState } from "../types";
import { NoteItem } from "./NoteItem";
import "./NotesView.css";

export const NotesView: React.FC<{
  settings: AppSettings | null;
  onUpdateSettings: (patch: Partial<AppSettings>) => Promise<boolean>;
  appState?: AppState;
  draft?: string;
  onDraftChange?: (value: string) => void;
  onDraftSaved?: (submitted: string) => void;
  externalRevision?: number;
  onNotesChanged?: () => void;
}> = ({
  settings,
  onUpdateSettings,
  appState = "READY",
  draft,
  onDraftChange,
  onDraftSaved,
  externalRevision = 0,
  onNotesChanged,
}) => {
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [query, setQuery] = useState("");
  const [localDraft, setLocalDraft] = useState("");
  const text = draft ?? localDraft;
  const wordCount = text.trim() ? text.trim().split(/\s+/).length : 0;
  const setText = onDraftChange ?? setLocalDraft;
  const [message, setMessage] = useState("");
  const [error, setError] = useState("");
  const [loadError, setLoadError] = useState("");
  const [loadedRequest, setLoadedRequest] = useState<string | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  const pendingRef = useRef(false);
  const [recordPending, setRecordPending] = useState(false);
  const recordPendingRef = useRef(false);
  const [revision, setRevision] = useState(0);
  const [page, setPage] = useState(0);
  const [total, setTotal] = useState(0);
  const [more, setMore] = useState(false);
  const requestKey = JSON.stringify([query.trim(), revision, externalRevision, page]);
  const loading = loadedRequest !== requestKey;
  const [folderDraft, setFolderDraft] = useState<string | null>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const searchRef = useRef<HTMLInputElement>(null);
  const previousState = useRef(appState);
  const recording = appState === "RECORDING";
  const processing = appState === "PROCESSING" || appState === "INJECTING";
  const unavailable = !["READY", "IDLE", "ERROR", "RECORDING"].includes(appState);
  const folder = folderDraft ?? settings?.notes_folder ?? "";

  useEffect(() => {
    if (appState === "READY" && previousState.current !== "READY")
      setRevision((value) => value + 1);
    previousState.current = appState;
  }, [appState]);

  useEffect(() => {
    let alive = true;
    const timer = setTimeout(() => {
      setLoadError("");
      void api
        .queryHistory({ kind: "note", query: query.trim(), limit: 50, offset: page * 50 })
        .then((result) => {
          if (!alive) return;
          if (!result.entries.length && page > 0) {
            setPage((value) => value - 1);
            return;
          }
          setEntries(result.entries);
          setTotal(result.total);
          setMore(result.next_offset !== null);
          if (result.recovery_notice) setLoadError(result.recovery_notice);
        })
        .catch((reason) => {
          if (alive) setLoadError(`Could not load notes. ${String(reason)}`);
        })
        .finally(() => {
          if (alive) setLoadedRequest(requestKey);
        });
    }, 180);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [query, revision, externalRevision, page, requestKey]);

  const refreshNotes = () => {
    if (onNotesChanged) onNotesChanged();
    else setRevision((value) => value + 1);
  };

  const act = async (name: string, action: () => Promise<void>) => {
    if (pendingRef.current) return;
    pendingRef.current = true;
    setPending(name);
    setMessage("");
    setError("");
    try {
      await action();
    } catch (reason) {
      setError(String(reason));
    } finally {
      pendingRef.current = false;
      setPending(null);
    }
  };
  const save = () =>
    act("save", async () => {
      await api.addNote(text.trim());
      if (onDraftSaved) onDraftSaved(text);
      else setText("");
      setPage(0);
      setQuery("");
      refreshNotes();
      setMessage("Note saved.");
      requestAnimationFrame(() => composerRef.current?.focus());
    });

  const toggleRecording = async () => {
    if (recordPendingRef.current) return;
    recordPendingRef.current = true;
    setRecordPending(true);
    setError("");
    try {
      if (recording) await api.stopRecording();
      else await api.startRecording("note");
    } catch (reason) {
      setError(String(reason));
    } finally {
      recordPendingRef.current = false;
      setRecordPending(false);
    }
  };

  return (
    <div className="workspace-page notes-page">
      <header className="page-heading notes-heading">
        <div>
          <h1>Notes</h1>
          <p>Write it down. Talk it through. Keep it here.</p>
        </div>
        <button
          className="btn btn-ghost"
          disabled={!!pending || (!query.trim() && (loading || total === 0))}
          onClick={() => {
            void act("export", async () => {
              setMessage(`Notes exported to ${await api.exportNotes()}`);
            });
          }}
        >
          {pending === "export" ? (
            <Loader2 className="w-4 h-4 animate-spin" aria-hidden />
          ) : (
            <ArrowDownToLine className="w-4 h-4" aria-hidden />
          )}
          {pending === "export" ? "Exporting…" : "Export Markdown"}
        </button>
      </header>
      {error && (
        <p role="alert" className="notes-feedback text-danger">
          {error}
        </p>
      )}
      {message && (
        <p role="status" className="notes-feedback text-muted">
          {message}
        </p>
      )}
      <div className="notes-layout">
        <section className="notes-compose" aria-labelledby="note-compose-heading">
          <div className="notes-composer">
            <div className="notes-composer-heading">
              <NotebookPen className="w-4 h-4 text-muted" aria-hidden />
              <h2 id="note-compose-heading">New note</h2>
            </div>
            <textarea
              ref={composerRef}
              className="notes-textarea"
              aria-label="New note"
              placeholder="What’s on your mind?"
              maxLength={100000}
              value={text}
              disabled={pending === "save"}
              onChange={(event) => setText(event.target.value)}
              onKeyDown={(event) => {
                if (
                  (event.ctrlKey || event.metaKey) &&
                  event.key === "Enter" &&
                  text.trim() &&
                  !pending
                ) {
                  event.preventDefault();
                  void save();
                }
              }}
            />
            <div className="notes-composer-footer">
              <span className="text-xs text-muted">
                {wordCount
                  ? `${wordCount} ${wordCount === 1 ? "word" : "words"}`
                  : "A little space for a thought."}
              </span>
              <button
                className="btn btn-primary"
                disabled={!!pending || !text.trim()}
                onClick={() => {
                  void save();
                }}
              >
                {pending === "save" ? (
                  <Loader2 className="w-3.5 h-3.5 animate-spin" aria-hidden />
                ) : (
                  <Check className="w-3.5 h-3.5" aria-hidden />
                )}
                {pending === "save" ? "Saving…" : "Save note"}
              </button>
            </div>
          </div>
          <div className={`notes-record ${recording ? "is-recording" : ""}`}>
            <div>
              <h2>
                {recording
                  ? "Recording your note"
                  : processing
                    ? "Turning voice into text"
                    : "Prefer to say it?"}
              </h2>
              <p>
                {recording
                  ? "Speak naturally, then stop to save your note."
                  : processing
                    ? "Your note will appear when it’s ready."
                    : "Voice notes are saved here, without pasting into another app."}
              </p>
            </div>
            <button
              className={`btn ${recording ? "btn-danger" : "btn-secondary"}`}
              disabled={recordPending || unavailable || processing}
              onClick={() => {
                void toggleRecording();
              }}
            >
              {recordPending || processing ? (
                <Loader2 className="w-4 h-4 animate-spin" aria-hidden />
              ) : recording ? (
                <Square className="w-3.5 h-3.5" aria-hidden />
              ) : (
                <Mic className="w-4 h-4" aria-hidden />
              )}
              {processing
                ? "Processing note…"
                : recordPending
                  ? recording
                    ? "Stopping…"
                    : "Starting…"
                  : recording
                    ? "Stop recording"
                    : "Record note"}
            </button>
          </div>
          <details className="notes-export-settings">
            <summary>
              <FolderOpen className="w-4 h-4" aria-hidden />
              <span>Export location</span>
              <ChevronDown className="w-3.5 h-3.5 notes-disclosure" aria-hidden />
            </summary>
            <p>Markdown exports go to Documents / Reflow Notes unless you choose another folder.</p>
            <label htmlFor="notes-export-folder">Export folder</label>
            <input
              id="notes-export-folder"
              className="field w-full"
              placeholder="Documents / Reflow Notes"
              value={folder}
              disabled={pending === "folder"}
              onChange={(event) => setFolderDraft(event.target.value)}
            />
            <button
              className="btn btn-ghost"
              disabled={!!pending || folder.trim() === (settings?.notes_folder ?? "")}
              onClick={() => {
                void act("folder", async () => {
                  if (!(await onUpdateSettings({ notes_folder: folder.trim() })))
                    throw new Error("Could not save export folder. Try again.");
                  setFolderDraft(null);
                  setMessage("Export folder saved.");
                });
              }}
            >
              {pending === "folder" ? "Saving…" : "Save folder"}
            </button>
          </details>
        </section>
        <section
          className="notes-library"
          aria-labelledby="saved-notes-heading"
          aria-busy={loading}
        >
          <div className="notes-library-heading">
            <h2 id="saved-notes-heading">Your notes</h2>
            <span className="text-xs text-muted">
              {loading ? "" : `${total} ${total === 1 ? "note" : "notes"}`}
            </span>
          </div>
          <div className="notes-search">
            <Search className="w-4 h-4 text-muted" aria-hidden />
            <input
              ref={searchRef}
              type="search"
              className="field"
              aria-label="Search notes"
              placeholder="Find a note…"
              value={query}
              onChange={(event) => {
                setQuery(event.target.value);
                setPage(0);
              }}
            />
            {query && (
              <button
                className="icon-btn"
                aria-label="Clear search"
                onClick={() => {
                  setQuery("");
                  setPage(0);
                  requestAnimationFrame(() => searchRef.current?.focus());
                }}
              >
                <X className="w-3.5 h-3.5" aria-hidden />
              </button>
            )}
          </div>
          {loadError && (
            <div className="notes-load-error">
              <p role="alert">{loadError}</p>
              <button
                className="btn btn-ghost"
                disabled={loading}
                onClick={() => setRevision((value) => value + 1)}
              >
                Retry
              </button>
            </div>
          )}
          {loading && !entries.length ? (
            <div className="notes-loading" role="status">
              <span className="sr-only">Loading notes…</span>
              <div />
              <div />
              <div />
            </div>
          ) : entries.length ? (
            <div className="notes-list">
              {entries.map((entry) => (
                <NoteItem key={entry.id} entry={entry} onChanged={refreshNotes} />
              ))}
            </div>
          ) : (
            !loadError && (
              <div className="notes-empty">
                <NotebookPen className="w-7 h-7 text-muted" aria-hidden />
                <h3>{query ? "No notes found" : "Make room for your ideas"}</h3>
                <p>
                  {query
                    ? "Try a different word or clear your search."
                    : "A quick thought, a voice memo, a plan for later. Write or record your first note."}
                </p>
                <button
                  className="btn btn-secondary"
                  onClick={() => {
                    if (query) {
                      setQuery("");
                      setPage(0);
                      requestAnimationFrame(() => searchRef.current?.focus());
                    } else composerRef.current?.focus();
                  }}
                >
                  {query ? "Clear search" : "Write a note"}
                </button>
              </div>
            )
          )}
          {(page > 0 || more) && (
            <nav className="notes-pagination" aria-label="Notes pages">
              <span className="text-xs text-muted">Page {page + 1}</span>
              <div>
                <button
                  className="icon-btn"
                  aria-label="Previous page"
                  disabled={!page || loading}
                  onClick={() => setPage((value) => value - 1)}
                >
                  <ChevronLeft className="w-4 h-4" aria-hidden />
                </button>
                <button
                  className="icon-btn"
                  aria-label="Next page"
                  disabled={!more || loading}
                  onClick={() => setPage((value) => value + 1)}
                >
                  <ChevronRight className="w-4 h-4" aria-hidden />
                </button>
              </div>
            </nav>
          )}
        </section>
      </div>
    </div>
  );
};
