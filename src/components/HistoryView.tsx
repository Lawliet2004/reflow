import React, { useEffect, useMemo, useRef, useState } from "react";
import { Search, Copy, Check, Trash2, Inbox, ShieldCheck, X, Clock, FileText } from "lucide-react";
import { HistoryEntry } from "../types";
import { api } from "../services/tauriApi";
import { hasAiEdit, historyCleaned, historyOriginal } from "../historyDisplay";

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

function matchesFilter(entry: HistoryEntry, filter: DayFilter): boolean {
  if (filter === "all") return true;
  const d = new Date(entry.created_at);
  const now = new Date();
  if (filter === "today") return d.toDateString() === now.toDateString();
  if (filter === "yesterday") {
    const y = new Date();
    y.setDate(now.getDate() - 1);
    return d.toDateString() === y.toDateString();
  }
  const cutoff = new Date();
  cutoff.setDate(now.getDate() - 1);
  cutoff.setHours(0, 0, 0, 0);
  return d.getTime() < cutoff.getTime();
}

export const HistoryView: React.FC = () => {
  const [query, setQuery] = useState("");
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const requestId = useRef(0);
  const [page, setPage] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [loading, setLoading] = useState(true);
  const [showOriginal, setShowOriginal] = useState<Record<string, boolean>>({});
  const [confirmClear, setConfirmClear] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [dayFilter, setDayFilter] = useState<DayFilter>("all");

  useEffect(() => {
    const id = ++requestId.current;
    const t = setTimeout(async () => {
      setLoading(true);
      setError(null);
      try {
        const rows = query.trim()
          ? await api.searchHistory(query.trim())
          : await api.getHistory(50, page * 50);
        if (id !== requestId.current) return;
        setEntries(rows);
        setHasMore(!query.trim() && rows.length === 50);
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
  }, [query, page]);

  useEffect(() => {
    if (!copiedId) return;
    const timer = setTimeout(() => setCopiedId(null), 1500);
    return () => clearTimeout(timer);
  }, [copiedId]);

  const visible = useMemo(
    () => entries.filter((e) => matchesFilter(e, dayFilter)),
    [entries, dayFilter],
  );

  const grouped = useMemo(() => {
    const map = new Map<string, HistoryEntry[]>();
    for (const e of visible) {
      const label = dayLabel(e.created_at);
      if (!map.has(label)) map.set(label, []);
      map.get(label)!.push(e);
    }
    return Array.from(map.entries());
  }, [visible]);

  const remove = async (id: string) => {
    try {
      await api.deleteHistoryItem(id);
      requestId.current += 1;
      setLoading(false);
      setEntries((prev) => prev.filter((e) => e.id !== id));
    } catch (e) {
      console.error("Failed to delete history item:", e);
      setError("Could not delete this transcript. Please try again.");
    }
  };

  const displayText = (entry: HistoryEntry) =>
    showOriginal[entry.id] ? historyOriginal(entry) : historyCleaned(entry);

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
    } catch (e) {
      console.error("Failed to clear history:", e);
      setError("Could not clear history. Your transcripts are still saved.");
    } finally {
      setClearing(false);
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
          <p className="eyebrow">YOUR WORDS, KEPT CLOSE</p>
          <h1 className="font-display font-semibold tracking-tight text-ink">
            A thought worth keeping.
          </h1>
          <p className="text-[12.5px] text-muted mt-0.5">
            Find, revisit, and reuse your dictations. Saved on this computer.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <span className="chip !bg-accent-soft !border-accent-border !text-accent">
            <ShieldCheck className="w-3.5 h-3.5 text-accent" />
            {entries.length} {entries.length === 1 ? "entry" : "entries"}
          </span>
          {entries.length > 0 && (
            <button
              className="btn btn-ghost !py-1.5 !px-3 !text-[12px]"
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

      {confirmClear && (
        <div className="panel p-4 mb-5 border-rose-500/30">
          <p className="text-[13.5px] font-semibold text-ink">Clear all history?</p>
          <p className="text-[12.5px] text-muted mt-1">
            This deletes every stored transcript on this computer. It cannot be undone.
          </p>
          <div className="flex items-center gap-2 mt-3">
            <button
              className="btn btn-danger !py-1.5 !px-3 !text-[12.5px]"
              onClick={clearAll}
              disabled={clearing}
            >
              <Trash2 className="w-3.5 h-3.5" />
              {clearing ? "Clearing…" : "Clear all"}
            </button>
            <button
              className="btn btn-ghost !py-1.5 !px-3 !text-[12.5px]"
              onClick={() => setConfirmClear(false)}
            >
              Cancel
            </button>
          </div>
        </div>
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
            onClick={() => setQuery("")}
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
              onClick={() => setDayFilter(c.id)}
              aria-pressed={active}
              className={`px-2.5 py-1 rounded-full text-[11.5px] font-semibold transition-colors cursor-pointer ${
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
          <p className="text-[13px] text-muted font-medium">Searching transcripts…</p>
        </div>
      ) : grouped.length === 0 ? (
        <div className="panel flex flex-col items-center py-16 text-center">
          <div className="w-12 h-12 rounded-2xl bg-accent-soft border border-accent-border flex items-center justify-center mb-3.5 text-accent shadow-xs">
            <Inbox className="w-6 h-6" strokeWidth={1.75} />
          </div>
          <p className="text-[14.5px] text-ink font-semibold">
            {query.trim() ? "No matching transcripts" : "No history recorded yet"}
          </p>
          <p className="text-[12.5px] text-muted mt-1 max-w-sm">
            {query.trim()
              ? `No transcripts match "${query}". Try searching for another phrase.`
              : "Hold your hotkey anywhere on your computer to speak — completed transcripts will appear here."}
          </p>
          {query.trim() && (
            <button
              onClick={() => setQuery("")}
              className="btn btn-ghost !py-1.5 !px-3.5 !text-[12px] mt-4"
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
                <span className="text-[11px] text-muted">({rows.length})</span>
              </div>
              <div className="panel divide-y divide-line overflow-hidden shadow-xs">
                {rows.map((entry) => {
                  const edited = hasAiEdit(entry);
                  const original = Boolean(showOriginal[entry.id]);
                  return (
                    <div
                      key={entry.id}
                      className="group px-4 py-3.5 hover:bg-base-2 transition-colors"
                    >
                      <div className="flex items-start justify-between gap-3">
                        <p
                          className="flex-1 text-[13.5px] leading-6 text-ink-2 font-normal"
                          title={displayText(entry)}
                        >
                          {displayText(entry)}
                        </p>
                        <div className="flex items-center gap-0.5 shrink-0">
                          <button
                            className="icon-btn hover:bg-base-2"
                            title="Copy to clipboard"
                            aria-label="Copy transcript"
                            onClick={() => copy(entry)}
                          >
                            {copiedId === entry.id ? (
                              <Check className="w-3.5 h-3.5 text-emerald-600 dark:text-emerald-400" />
                            ) : (
                              <Copy className="w-3.5 h-3.5 text-muted" />
                            )}
                          </button>
                          <button
                            className="icon-btn hover:!bg-rose-500/10 hover:!text-rose-500"
                            title="Delete entry"
                            aria-label="Delete entry"
                            onClick={() => remove(entry.id)}
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </button>
                        </div>
                      </div>

                      <div className="flex items-center gap-2.5 mt-2 text-[11.5px] text-muted flex-wrap">
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
                            <span className="truncate max-w-[180px] px-1.5 py-0.5 rounded bg-surface-3 text-ink-2 text-[11px] font-medium border border-line">
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
                              className="text-[11px] font-semibold text-accent hover:text-accent-hover cursor-pointer"
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
      {!query.trim() && (page > 0 || hasMore) && (
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
