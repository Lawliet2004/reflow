import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Search,
  Copy,
  Check,
  Trash2,
  Inbox,
  ShieldCheck,
  X,
  Clock,
  FileText,
  MoreVertical,
  FileAudio,
  RefreshCw,
  Loader2,
  Pencil,
  Pin,
  Tags,
  Sparkles,
  Download,
} from "lucide-react";
import { HistoryEntry } from "../types";
import { api } from "../services/tauriApi";
import { createEventScope } from "../services/eventScope";
import { hasAiEdit, historyCleaned, historyOriginal } from "../historyDisplay";
import { TranscriptMenu } from "./TranscriptMenu";

function dayLabel(iso: string): string {
  const d = new Date(iso);
  const today = new Date();
  const yesterday = new Date();
  yesterday.setDate(today.getDate() - 1);
  const same = (a: Date, b: Date) => a.toDateString() === b.toDateString();
  if (same(d, today)) return "Today";
  if (same(d, yesterday)) return "Yesterday";
  return d.toLocaleDateString(undefined, {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
}

function timeLabel(iso: string): string {
  return new Date(iso).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
  });
}

function durationLabel(ms: number): string {
  if (!ms || ms < 1000) return "<1s";
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const r = s % 60;
  return r ? `${m}m ${r}s` : `${m}m`;
}

type DayFilter = "all" | "today" | "yesterday" | "earlier";

export function historyDateRange(filter: DayFilter): { from?: string; until?: string } {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const yesterday = new Date(today);
  yesterday.setDate(today.getDate() - 1);
  const tomorrow = new Date(today);
  tomorrow.setDate(today.getDate() + 1);
  switch (filter) {
    case "today":
      return { from: today.toISOString(), until: tomorrow.toISOString() };
    case "yesterday":
      return { from: yesterday.toISOString(), until: today.toISOString() };
    case "earlier":
      return { until: yesterday.toISOString() };
    default:
      return {};
  }
}

export const HistoryView: React.FC = () => {
  const [summary, setSummary] = useState<{ id: string; text: string } | null>(null);
  const [summaryBusy, setSummaryBusy] = useState<string | null>(null);
  const [summarySaved, setSummarySaved] = useState(false);
  const [tagEntry, setTagEntry] = useState<HistoryEntry | null>(null);
  const [tagDraft, setTagDraft] = useState("");
  const [editing, setEditing] = useState<string | null>(null);
  const editEpoch = useRef(0);
  const tagEpoch = useRef(0);
  const [draft, setDraft] = useState("");
  const [revision, setRevision] = useState(0);
  const [kind, setKind] = useState("");
  const [pinnedOnly, setPinnedOnly] = useState(false);
  const [app, setApp] = useState("");
  const [tag, setTag] = useState("");
  const [query, setQuery] = useState("");
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const requestId = useRef(0);
  const [page, setPage] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [total, setTotal] = useState(0);
  const [recoveryNotice, setRecoveryNotice] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const [exportPath, setExportPath] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [showOriginal, setShowOriginal] = useState<Record<string, boolean>>({});
  const [confirmClear, setConfirmClear] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [dayFilter, setDayFilter] = useState<DayFilter>("all");
  const [openMenu, setOpenMenu] = useState<{
    id: string;
    trigger: HTMLButtonElement;
    focusLast?: boolean;
  } | null>(null);
  const menuId = openMenu?.id;
  const closeMenu = useCallback(
    (restoreFocus = true) => {
      setOpenMenu(null);
      if (restoreFocus) openMenu?.trigger.focus({ preventScroll: true });
    },
    [openMenu],
  );
  const [busyId, setBusyId] = useState<string | null>(null);
  const [audioPath, setAudioPath] = useState<string | null>(null);
  const [now, setNow] = useState(Date.now);

  useEffect(() => {
    const scope = createEventScope();
    void scope.listen<HistoryEntry>("history:updated", () => {
      requestId.current += 1;
      setRevision((value) => value + 1);
    });
    return () => scope.dispose();
  }, []);

  // Expire actions even when the user leaves the history page/menu open.
  useEffect(() => {
    const expiries = entries.flatMap((entry) =>
      entry.audio_available && entry.audio_expires_at && entry.audio_expires_at > now
        ? [entry.audio_expires_at]
        : [],
    );
    if (!expiries.length) return;
    const timer = setTimeout(
      () => setNow(Date.now()),
      Math.max(0, Math.min(Math.min(...expiries) - Date.now() + 1, 2_147_483_647)),
    );
    return () => clearTimeout(timer);
  }, [entries, now]);

  useEffect(() => {
    const id = ++requestId.current;
    const t = setTimeout(async () => {
      setLoading(true);
      setError(null);
      try {
        const result = await api.queryHistory({
          query: query.trim(),
          kind: kind || undefined,
          pinned_only: pinnedOnly,
          application_process: app || undefined,
          tag: tag || undefined,
          ...historyDateRange(dayFilter),
          limit: 50,
          offset: page * 50,
        });
        if (id !== requestId.current) return;
        setEntries(result.entries);
        setTotal(result.total);
        setRecoveryNotice(result.recovery_notice);
        setHasMore(result.next_offset !== null);
      } catch {
        if (id === requestId.current) setError("Could not load history. Try searching again.");
      } finally {
        if (id === requestId.current) setLoading(false);
      }
    }, 180);
    return () => {
      clearTimeout(t);
      requestId.current += 1;
    };
  }, [query, page, dayFilter, kind, pinnedOnly, app, tag, revision]);

  useEffect(() => {
    if (!copiedId) return;
    const timer = setTimeout(() => setCopiedId(null), 1500);
    return () => clearTimeout(timer);
  }, [copiedId]);

  const grouped = useMemo(() => {
    const map = new Map<string, HistoryEntry[]>();
    for (const e of entries) {
      const label = dayLabel(e.created_at);
      if (!map.has(label)) map.set(label, []);
      map.get(label)!.push(e);
    }
    return Array.from(map.entries());
  }, [entries]);

  const refreshHistory = () => {
    requestId.current += 1;
    setRevision((value) => value + 1);
  };

  const remove = async (id: string) => {
    try {
      await api.deleteHistoryItem(id);
      refreshHistory();
    } catch (e) {
      console.error("Failed to delete history item:", e);
      setError("Could not delete this transcript. Please try again.");
    }
  };

  const displayText = (entry: HistoryEntry) =>
    showOriginal[entry.id] ? historyOriginal(entry) : historyCleaned(entry);

  const replaceEntry = (updated: HistoryEntry | null) => {
    if (updated) {
      setEntries((prev) => prev.map((e) => (e.id === updated.id ? updated : e)));
      refreshHistory();
    }
  };

  const extractAudio = async (entry: HistoryEntry) => {
    setBusyId(entry.id);
    setAudioPath(null);
    setError(null);
    try {
      setAudioPath(await api.extractHistoryAudio(entry.id));
    } catch (error) {
      setError(`Could not extract audio: ${String(error)}`);
    } finally {
      setBusyId(null);
    }
  };

  const retry = async (entry: HistoryEntry) => {
    setBusyId(entry.id);
    setError(null);
    try {
      const updated = await api.retryHistoryTranscript(entry.id);
      if (updated) replaceEntry(updated);
      else setError("Could not retry this transcript. It may no longer exist.");
    } catch (e) {
      setError(`Could not retry this transcript: ${String(e)}`);
    } finally {
      setBusyId(null);
    }
  };

  const copy = async (entry: HistoryEntry) => {
    try {
      await navigator.clipboard.writeText(displayText(entry));
      setCopiedId(entry.id);
    } catch {
      setError("Could not copy. Select the transcript and copy it manually.");
    }
  };

  const clearAll = async () => {
    setClearing(true);
    try {
      await api.clearAllHistory();
      requestId.current += 1;
      setLoading(false);
      setEntries([]);
      setConfirmClear(false);
      setPage(0);
      setHasMore(false);
      setTotal(0);
    } catch (e) {
      console.error("Failed to clear history:", e);
      setError("Could not clear history. Your transcripts are still saved.");
    } finally {
      setClearing(false);
    }
  };

  const exportHistory = async () => {
    setExporting(true);
    setError(null);
    setExportPath(null);
    try {
      setExportPath(
        await api.exportHistory({
          query: query.trim(),
          kind: kind || undefined,
          pinned_only: pinnedOnly,
          application_process: app || undefined,
          tag: tag || undefined,
          ...historyDateRange(dayFilter),
        }),
      );
    } catch (error) {
      setError(`Could not export history: ${String(error)}`);
    } finally {
      setExporting(false);
    }
  };

  const filterChips: { id: DayFilter; label: string }[] = [
    { id: "all", label: "All" },
    { id: "today", label: "Today" },
    { id: "yesterday", label: "Yesterday" },
    { id: "earlier", label: "Earlier" },
  ];

  return (
    <div className="workspace-page history-page animate-fade-rise">
      <header className="page-heading flex-wrap">
        <div>
          <h1 className="text-ink">History</h1>
          <p>Find, revisit, and reuse your dictations. Saved on this computer.</p>
        </div>
        <div className="flex items-center gap-2">
          <button
            className="btn btn-ghost"
            disabled={exporting || total === 0}
            onClick={exportHistory}
          >
            {exporting ? "Exporting…" : "Export matching"}
          </button>
          <span className="chip !bg-accent-soft !border-accent-border !text-accent">
            <ShieldCheck className="w-3.5 h-3.5 text-accent" />
            {total} {total === 1 ? "entry" : "entries"}
          </span>
          {entries.length > 0 && (
            <button
              className="btn btn-ghost !py-1.5 !px-3 !text-xs"
              onClick={() => setConfirmClear(true)}
            >
              Clear all
            </button>
          )}
        </div>
      </header>
      {error && (
        <p role="alert" className="text-sm text-danger mb-4">
          {error}
        </p>
      )}
      {recoveryNotice && (
        <p role="alert" className="text-sm text-danger mb-4">
          {recoveryNotice}
        </p>
      )}
      {exportPath && (
        <p role="status" className="text-sm text-muted mb-4 break-all">
          History exported to {exportPath}
        </p>
      )}
      {audioPath && (
        <p role="status" className="text-sm text-muted mb-4 break-all">
          Audio exported to {audioPath}
        </p>
      )}

      {confirmClear && (
        <div className="panel p-4 mb-5 border-danger/30">
          <p className="text-sm font-semibold text-ink">Clear all history?</p>
          <p className="text-sm text-muted mt-1">
            This deletes every stored transcript on this computer. It cannot be undone.
          </p>
          <div className="flex items-center gap-2 mt-3">
            <button
              className="btn btn-danger !py-1.5 !px-3 !text-sm"
              onClick={clearAll}
              disabled={clearing}
            >
              <Trash2 className="w-3.5 h-3.5" />
              {clearing ? "Clearing…" : "Clear all"}
            </button>
            <button
              className="btn btn-ghost !py-1.5 !px-3 !text-sm"
              onClick={() => setConfirmClear(false)}
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {tagEntry && (
        <section className="panel p-4 mb-4 space-y-2" aria-label="Edit tags">
          <label className="text-sm">
            Tags (comma separated)
            <input
              className="field w-full"
              aria-label="Transcript tags"
              value={tagDraft}
              onChange={(e) => {
                tagEpoch.current += 1;
                setTagDraft(e.target.value);
              }}
            />
          </label>
          <button
            className="btn btn-primary"
            onClick={async () => {
              const epoch = ++tagEpoch.current;
              try {
                await api.updateHistoryMetadata(
                  tagEntry.id,
                  tagEntry.pinned ?? false,
                  tagDraft
                    .split(",")
                    .map((v) => v.trim())
                    .filter(Boolean)
                    .join(","),
                );
                if (epoch === tagEpoch.current) setTagEntry(null);
                refreshHistory();
              } catch (e) {
                setError(String(e));
              }
            }}
          >
            Save tags
          </button>
          <button
            className="btn btn-ghost"
            onClick={() => {
              tagEpoch.current += 1;
              setTagEntry(null);
            }}
          >
            Cancel tags
          </button>
        </section>
      )}
      <div className="flex flex-wrap gap-2 mb-3">
        <select
          aria-label="Entry kind"
          className="field"
          value={kind}
          onChange={(e) => {
            setKind(e.target.value);
            setPage(0);
          }}
        >
          <option value="">All kinds</option>
          {["dictation", "command", "assistant", "note", "file", "meeting"].map((value) => (
            <option key={value}>{value}</option>
          ))}
        </select>
        <label className="flex items-center gap-2 text-xs">
          <input
            type="checkbox"
            checked={pinnedOnly}
            onChange={(e) => {
              setPinnedOnly(e.target.checked);
              setPage(0);
            }}
          />
          Pinned only
        </label>
        <input
          className="field"
          aria-label="Filter application"
          placeholder="Application process"
          value={app}
          onChange={(e) => {
            setApp(e.target.value);
            setPage(0);
          }}
        />
        <input
          className="field"
          aria-label="Filter tag"
          placeholder="Tag"
          value={tag}
          onChange={(e) => {
            setTag(e.target.value);
            setPage(0);
          }}
        />
      </div>
      {summary && (
        <section className="panel p-4 mb-4" aria-label="Transcript summary">
          <div className="flex justify-between items-center mb-2">
            <h2 className="font-semibold">Transcript summary</h2>
            <button
              className="btn btn-ghost"
              aria-label="Close summary"
              onClick={() => setSummary(null)}
            >
              ×
            </button>
          </div>
          <p className="text-sm whitespace-pre-wrap max-h-80 overflow-y-auto">{summary.text}</p>
          <div className="flex gap-2 mt-3">
            <button
              className="btn btn-secondary"
              onClick={async () => {
                try {
                  await navigator.clipboard.writeText(summary.text);
                } catch (e) {
                  setError(String(e));
                }
              }}
            >
              Copy summary
            </button>
            <button
              className="btn btn-secondary"
              disabled={summarySaved}
              onClick={async () => {
                try {
                  await api.addNote(summary.text);
                  setSummarySaved(true);
                } catch (e) {
                  setError(String(e));
                }
              }}
            >
              {summarySaved ? "Saved in Notes" : "Save summary as note"}
            </button>
          </div>
        </section>
      )}
      <div className="relative mb-3">
        <Search className="absolute left-3.5 top-1/2 -translate-y-1/2 w-4 h-4 text-muted pointer-events-none" />
        <input
          value={query}
          onChange={(e) => {
            setPage(0);
            setQuery(e.target.value);
          }}
          aria-label="Search transcripts"
          placeholder="Search all transcripts…"
          className="field w-full !pl-10 !pr-9 !py-2.5 shadow-xs"
        />
        {query && (
          <button
            onClick={() => {
              setPage(0);
              setQuery("");
            }}
            className="absolute right-3 top-1/2 -translate-y-1/2 text-muted hover:text-ink p-0.5"
            title="Clear search"
            aria-label="Clear search"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        )}
      </div>

      <div className="flex items-center gap-1.5 mb-5">
        {filterChips.map((c) => {
          const active = dayFilter === c.id;
          return (
            <button
              key={c.id}
              onClick={() => {
                setPage(0);
                setDayFilter(c.id);
              }}
              aria-pressed={active}
              className={`px-2.5 py-1 rounded-full text-xs font-semibold transition-colors cursor-pointer ${
                active
                  ? "bg-accent-soft text-accent border border-accent-border"
                  : "bg-surface text-muted border border-line hover:text-ink hover:bg-base-2"
              }`}
            >
              {c.label}
            </button>
          );
        })}
      </div>

      {loading && entries.length === 0 ? (
        <div className="py-16 text-center">
          <div className="w-5 h-5 border-2 border-accent border-t-transparent rounded-full animate-spin mx-auto mb-3" />
          <p className="text-sm text-muted font-medium">Searching transcripts…</p>
        </div>
      ) : grouped.length === 0 ? (
        <div className="panel flex flex-col items-center py-16 text-center">
          <div className="w-12 h-12 rounded-2xl bg-accent-soft border border-accent-border flex items-center justify-center mb-3.5 text-accent shadow-xs">
            <Inbox className="w-6 h-6" strokeWidth={1.75} />
          </div>
          <p className="text-base text-ink font-semibold">
            {query.trim() ? "No matching transcripts" : "No history recorded yet"}
          </p>
          <p className="text-sm text-muted mt-1 max-w-sm">
            {query.trim()
              ? `No transcripts match "${query}". Try searching for another phrase.`
              : "Hold your hotkey anywhere on your computer to speak — completed transcripts will appear here."}
          </p>
          {query.trim() && (
            <button
              onClick={() => {
                setPage(0);
                setQuery("");
              }}
              className="btn btn-ghost !py-1.5 !px-3.5 !text-xs mt-4"
            >
              Clear search
            </button>
          )}
        </div>
      ) : (
        <div className="space-y-6">
          {grouped.map(([label, rows]) => (
            <section key={label}>
              <div className="flex items-center gap-2 mb-2 px-1">
                <span className="label-micro text-accent">{label}</span>
                <span className="text-2xs text-muted">({rows.length})</span>
              </div>
              <div className="panel divide-y divide-line overflow-hidden shadow-xs">
                {rows.map((entry) => {
                  const edited = hasAiEdit(entry);
                  const original = Boolean(showOriginal[entry.id]);
                  const busy = busyId === entry.id;
                  const hasAudio =
                    entry.audio_available &&
                    (entry.audio_expires_at == null || entry.audio_expires_at > now);
                  return (
                    <div
                      key={entry.id}
                      className="group px-4 py-3.5 hover:bg-base-2 transition-colors"
                    >
                      {summaryBusy === entry.id && (
                        <p role="status" className="text-xs text-accent mb-2">
                          Summarizing locally…
                        </p>
                      )}
                      {entry.pinned && <span className="text-xs text-accent">Pinned</span>}
                      {!!entry.tags && (
                        <div className="flex gap-1 mb-2">
                          {entry.tags
                            .split(",")
                            .filter(Boolean)
                            .map((tag) => (
                              <span
                                key={tag}
                                className="text-xs px-2 py-0.5 bg-base-2 rounded-full"
                              >
                                {tag}
                              </span>
                            ))}
                        </div>
                      )}
                      <div className="flex items-start justify-between gap-3">
                        {editing === entry.id ? (
                          <div className="flex-1 space-y-2">
                            <textarea
                              aria-label="Edit transcript"
                              className="field w-full min-h-24"
                              value={draft}
                              onChange={(e) => {
                                editEpoch.current += 1;
                                setDraft(e.target.value);
                              }}
                            />
                            <button
                              className="btn btn-primary"
                              disabled={busy}
                              onClick={async () => {
                                const epoch = ++editEpoch.current;
                                setBusyId(entry.id);
                                try {
                                  const updated = await api.editHistoryTranscript(entry.id, draft);
                                  replaceEntry(updated);
                                  if (epoch === editEpoch.current) setEditing(null);
                                } catch (e) {
                                  setError(String(e));
                                } finally {
                                  setBusyId(null);
                                }
                              }}
                            >
                              Save edit
                            </button>
                            <button
                              className="btn btn-ghost"
                              onClick={() => {
                                editEpoch.current += 1;
                                setEditing(null);
                              }}
                            >
                              Cancel
                            </button>
                          </div>
                        ) : (
                          <p
                            className="min-w-0 flex-1 text-sm leading-6 text-ink-2 font-normal whitespace-pre-wrap break-words"
                            title={displayText(entry)}
                          >
                            {displayText(entry)}
                          </p>
                        )}
                        <div className="flex items-center gap-0.5 shrink-0">
                          <button
                            className="icon-btn hover:bg-base-2"
                            title="Copy to clipboard"
                            aria-label="Copy transcript"
                            onClick={() => copy(entry)}
                          >
                            {copiedId === entry.id ? (
                              <Check className="w-3.5 h-3.5 text-success" />
                            ) : (
                              <Copy className="w-3.5 h-3.5 text-muted" />
                            )}
                          </button>
                          <div className="relative">
                            <button
                              className="icon-btn"
                              title="More options"
                              aria-label="More options"
                              aria-haspopup="menu"
                              aria-expanded={menuId === entry.id}
                              aria-controls={
                                menuId === entry.id ? `transcript-menu-${entry.id}` : undefined
                              }
                              onClick={(event) => {
                                setNow(Date.now());
                                const trigger = event.currentTarget;
                                setOpenMenu((current) =>
                                  current?.id === entry.id ? null : { id: entry.id, trigger },
                                );
                              }}
                              onKeyDown={(event) => {
                                if (event.key === "ArrowDown" || event.key === "ArrowUp") {
                                  event.preventDefault();
                                  setNow(Date.now());
                                  setOpenMenu({
                                    id: entry.id,
                                    trigger: event.currentTarget,
                                    focusLast: event.key === "ArrowUp",
                                  });
                                }
                              }}
                            >
                              {busy ? (
                                <Loader2 className="w-3.5 h-3.5 animate-spin" />
                              ) : (
                                <MoreVertical className="w-3.5 h-3.5" />
                              )}
                            </button>
                            {openMenu?.id === entry.id && (
                              <TranscriptMenu
                                id={`transcript-menu-${entry.id}`}
                                trigger={openMenu.trigger}
                                focusLast={openMenu.focusLast}
                                onClose={closeMenu}
                              >
                                <button
                                  className="row-menu-item"
                                  role="menuitem"
                                  onClick={() => {
                                    editEpoch.current += 1;
                                    setEditing(entry.id);
                                    setDraft(entry.final_transcript);
                                    closeMenu();
                                  }}
                                >
                                  <Pencil />
                                  Edit transcript
                                </button>
                                <button
                                  className="row-menu-item"
                                  role="menuitem"
                                  onClick={async () => {
                                    closeMenu();
                                    try {
                                      await api.updateHistoryMetadata(
                                        entry.id,
                                        !entry.pinned,
                                        entry.tags ?? "",
                                      );
                                      refreshHistory();
                                    } catch (e) {
                                      setError(String(e));
                                    }
                                  }}
                                >
                                  <Pin />
                                  {entry.pinned ? "Unpin transcript" : "Pin transcript"}
                                </button>
                                {entry.source === "file" &&
                                  ["txt", "srt", "vtt"].map((format) => (
                                    <button
                                      key={format}
                                      className="row-menu-item"
                                      role="menuitem"
                                      onClick={async () => {
                                        closeMenu();
                                        try {
                                          setExportPath(
                                            await api.exportFileTranscript(entry.id, format),
                                          );
                                        } catch (e) {
                                          setError(String(e));
                                        }
                                      }}
                                    >
                                      <Download />
                                      Export {format.toUpperCase()}
                                    </button>
                                  ))}
                                <button
                                  className="row-menu-item"
                                  role="menuitem"
                                  disabled={summaryBusy !== null}
                                  onClick={async () => {
                                    closeMenu();
                                    setSummaryBusy(entry.id);
                                    setError(null);
                                    try {
                                      setSummary({
                                        id: entry.id,
                                        text: await api.summarizeHistory(entry.id),
                                      });
                                      setSummarySaved(false);
                                    } catch (e) {
                                      setError(String(e));
                                    } finally {
                                      setSummaryBusy(null);
                                    }
                                  }}
                                >
                                  <Sparkles />
                                  Summarize transcript
                                </button>
                                <button
                                  className="row-menu-item"
                                  role="menuitem"
                                  onClick={() => {
                                    tagEpoch.current += 1;
                                    setTagEntry(entry);
                                    setTagDraft(entry.tags ?? "");
                                    closeMenu();
                                  }}
                                >
                                  <Tags />
                                  Edit tags
                                </button>
                                {hasAudio && (
                                  <button
                                    role="menuitem"
                                    className="row-menu-item"
                                    disabled={busy}
                                    onClick={() => {
                                      closeMenu();
                                      retry(entry);
                                    }}
                                  >
                                    <RefreshCw />
                                    Retry transcript
                                  </button>
                                )}
                                {hasAudio && (
                                  <button
                                    role="menuitem"
                                    className="row-menu-item"
                                    disabled={busy}
                                    onClick={() => {
                                      closeMenu();
                                      extractAudio(entry);
                                    }}
                                  >
                                    <FileAudio />
                                    Extract audio
                                  </button>
                                )}
                                <div role="separator" />
                                <button
                                  role="menuitem"
                                  className="row-menu-item is-danger"
                                  disabled={busy}
                                  onClick={() => {
                                    closeMenu();
                                    remove(entry.id);
                                  }}
                                >
                                  <Trash2 />
                                  Delete transcript
                                </button>
                              </TranscriptMenu>
                            )}
                          </div>
                        </div>
                      </div>

                      <div className="flex items-center gap-2.5 mt-2 text-xs text-muted flex-wrap">
                        <span className="flex items-center gap-1">
                          <Clock className="w-3 h-3 text-muted" />
                          {timeLabel(entry.created_at)}
                        </span>
                        {entry.duration_ms > 0 && (
                          <>
                            <span>·</span>
                            <span className="font-medium text-ink-2">
                              {durationLabel(entry.duration_ms)}
                            </span>
                          </>
                        )}
                        {entry.application_name && (
                          <>
                            <span>·</span>
                            <span className="truncate max-w-[180px] px-1.5 py-0.5 rounded bg-surface-3 text-ink-2 text-2xs font-medium border border-line">
                              {entry.application_name}
                            </span>
                          </>
                        )}
                        <span>·</span>
                        <span className="flex items-center gap-1">
                          <FileText className="w-3 h-3 text-muted" />
                          {entry.word_count} {entry.word_count === 1 ? "word" : "words"}
                        </span>
                        {edited && (
                          <>
                            <span>·</span>
                            <button
                              className="text-2xs font-semibold text-accent hover:text-accent-hover cursor-pointer"
                              onClick={() =>
                                setShowOriginal((prev) => ({
                                  ...prev,
                                  [entry.id]: !original,
                                }))
                              }
                            >
                              {original ? "Show cleaned" : "Show original"}
                            </button>
                          </>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            </section>
          ))}
        </div>
      )}
      {(page > 0 || hasMore) && (
        <nav aria-label="History pages" className="flex items-center justify-between mt-6">
          <button
            className="btn btn-ghost"
            disabled={page === 0 || loading}
            onClick={() => setPage((n) => n - 1)}
          >
            Previous
          </button>
          <span className="text-sm text-muted">Page {page + 1}</span>
          <button
            className="btn btn-ghost"
            disabled={!hasMore || loading}
            onClick={() => setPage((n) => n + 1)}
          >
            Next
          </button>
        </nav>
      )}
    </div>
  );
};
