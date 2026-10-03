import React, { useEffect, useRef, useState } from "react";
import { Check, Copy, Loader2, Mic, Pencil, Pin, Trash2 } from "lucide-react";
import { HistoryEntry } from "../types";
import { api } from "../services/tauriApi";

export function NoteItem({ entry, onChanged }: { entry: HistoryEntry; onChanged: () => void }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(entry.final_transcript);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [pending, setPending] = useState<string | null>(null);
  const pendingRef = useRef(false);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const editRef = useRef<HTMLTextAreaElement>(null);
  const editButtonRef = useRef<HTMLButtonElement>(null);
  const deleteButtonRef = useRef<HTMLButtonElement>(null);
  const keepButtonRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    if (confirmDelete) keepButtonRef.current?.focus();
  }, [confirmDelete]);
  useEffect(() => {
    if (editing) editRef.current?.focus();
  }, [editing]);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1600);
    return () => clearTimeout(timer);
  }, [copied]);
  const act = async (name: string, action: () => Promise<void>) => {
    if (pendingRef.current) return;
    pendingRef.current = true;
    setPending(name);
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
  const closeEdit = () => {
    setEditing(false);
    requestAnimationFrame(() => editButtonRef.current?.focus());
  };
  return (
    <article className={`note-item ${entry.pinned ? "is-pinned" : ""}`}>
      <div className="note-meta">
        <time dateTime={entry.created_at}>
          {new Date(entry.created_at).toLocaleDateString(undefined, {
            month: "short",
            day: "numeric",
          })}
          <span>
            {" "}
            ·{" "}
            {new Date(entry.created_at).toLocaleTimeString(undefined, {
              hour: "numeric",
              minute: "2-digit",
            })}
          </span>
        </time>
        <div>
          {entry.duration_ms > 0 && (
            <span>
              <Mic className="w-3 h-3" aria-hidden />
              Voice note
            </span>
          )}
          {entry.pinned && (
            <span className="text-ink-2">
              <Pin className="w-3 h-3 text-accent" aria-hidden />
              Pinned
            </span>
          )}
        </div>
      </div>
      {editing ? (
        <div className="note-editor">
          <textarea
            ref={editRef}
            className="field w-full"
            aria-label="Edit note text"
            maxLength={100000}
            value={draft}
            disabled={!!pending}
            onChange={(event) => setDraft(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Escape" && !pending) closeEdit();
            }}
          />
          <div className="flex flex-wrap gap-2">
            <button
              className="btn btn-primary"
              disabled={!!pending || !draft.trim()}
              onClick={() => {
                void act("edit", async () => {
                  await api.editHistoryTranscript(entry.id, draft.trim());
                  closeEdit();
                  onChanged();
                });
              }}
            >
              {pending === "edit" ? "Saving…" : "Save changes"}
            </button>
            <button className="btn btn-ghost" disabled={!!pending} onClick={closeEdit}>
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <p className="note-content">{entry.final_transcript}</p>
      )}
      <div className="note-actions">
        <button
          className={`icon-btn ${entry.pinned ? "text-accent" : ""}`}
          title={entry.pinned ? "Unpin note" : "Pin note"}
          aria-label={entry.pinned ? "Unpin note" : "Pin note"}
          aria-pressed={!!entry.pinned}
          disabled={!!pending}
          onClick={() => {
            void act("pin", async () => {
              await api.updateHistoryMetadata(entry.id, !entry.pinned, entry.tags ?? "");
              onChanged();
            });
          }}
        >
          <Pin className="w-3.5 h-3.5" aria-hidden />
        </button>
        <button
          className="icon-btn"
          title={copied ? "Copied" : "Copy note"}
          aria-label={copied ? "Copied" : "Copy note"}
          disabled={!!pending}
          onClick={() => {
            void act("copy", async () => {
              await navigator.clipboard.writeText(entry.final_transcript);
              setCopied(true);
            });
          }}
        >
          {copied ? (
            <Check className="w-3.5 h-3.5 text-success" aria-hidden />
          ) : (
            <Copy className="w-3.5 h-3.5" aria-hidden />
          )}
        </button>
        <button
          ref={editButtonRef}
          className="icon-btn"
          title="Edit note"
          aria-label="Edit note"
          disabled={!!pending || editing}
          onClick={() => {
            setDraft(entry.final_transcript);
            setError("");
            setConfirmDelete(false);
            setEditing(true);
          }}
        >
          <Pencil className="w-3.5 h-3.5" aria-hidden />
        </button>
        <button
          ref={deleteButtonRef}
          hidden={confirmDelete}
          className="icon-btn note-delete"
          title="Delete note"
          aria-label="Delete note"
          disabled={!!pending || editing || confirmDelete}
          onClick={() => {
            setError("");
            setConfirmDelete(true);
          }}
        >
          <Trash2 className="w-3.5 h-3.5" aria-hidden />
        </button>
        {pending && (
          <Loader2 className="w-3.5 h-3.5 text-muted animate-spin" aria-label="Updating note" />
        )}
        {copied && (
          <span role="status" className="text-xs text-success">
            Copied
          </span>
        )}
      </div>
      {confirmDelete && (
        <div className="note-delete-confirm">
          <p>Delete note?</p>
          <span>This permanently removes it from Notes and History.</span>
          <div className="flex flex-wrap gap-2">
            <button
              ref={keepButtonRef}
              className="btn btn-ghost"
              disabled={!!pending}
              onClick={() => {
                setConfirmDelete(false);
                requestAnimationFrame(() => deleteButtonRef.current?.focus());
              }}
            >
              Keep note
            </button>
            <button
              className="btn btn-danger"
              disabled={!!pending}
              onClick={() => {
                void act("delete", async () => {
                  if (!(await api.deleteHistoryItem(entry.id)))
                    throw new Error("This note could not be deleted. Reload and try again.");
                  onChanged();
                });
              }}
            >
              {pending === "delete" ? "Deleting…" : "Delete note"}
            </button>
          </div>
        </div>
      )}
      {error && (
        <p role="alert" className="note-error">
          {error}
        </p>
      )}
    </article>
  );
}
