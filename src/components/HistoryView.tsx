import React, { useEffect, useMemo, useState } from "react";
import {
  Search,
  Copy,
  Check,
  CornerDownLeft,
  Trash2,
  Inbox,
  ShieldCheck,
  X,
  Clock,
  FileText,
  Undo2,
} from "lucide-react";
import { HistoryEntry } from "../types";
import { api } from "../services/tauriApi";
import { hasAiEdit, historyCleaned, historyOriginal, undoAiText } from "../historyDisplay";

interface HistoryViewProps {
  onInjectText: (text: string) => void;
}

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
  cutoff.setDate(now.getDate() - 2);
  cutoff.setHours(0, 0, 0, 0);
  return d.getTime() < cutoff.getTime();
}

export const HistoryView: React.FC<HistoryViewProps> = ({ onInjectText }) => {
  const [query, setQuery] = useState("");
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [copiedId, setCopiedId] = useState<string | null>(null);
  const [injectedId, setInjectedId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [showOriginal, setShowOriginal] = useState<Record<string, boolean>>({});
  const [confirmClear, setConfirmClear] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [dayFilter, setDayFilter] = useState<DayFilter>("all");

  const load = async (q: string) => {
    setLoading(true);
    try {
      const rows = q.trim()
        ? await api.searchHistory(q.trim())
        : await api.getHistory(200, 0);
      setEntries(rows);
    } catch (e) {
      console.error("Failed to load history:", e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    const t = setTimeout(() => load(query), 180);
    return () => clearTimeout(t);
  }, [query]);

  const visible = useMemo(
    () => entries.filter((e) => matchesFilter(e, dayFilter)),
    [entries, dayFilter]
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
    setEntries((prev) => prev.filter((e) => e.id !== id));
    try {
      await api.deleteHistoryItem(id);
    } catch (e) {
      console.error("Failed to delete history item:", e);
    }
  };

  const displayText = (entry: HistoryEntry) =>
    showOriginal[entry.id] ? historyOriginal(entry) : historyCleaned(entry);

  const copy = (entry: HistoryEntry) => {
    navigator.clipboard.writeText(displayText(entry));
    setCopiedId(entry.id);
    setTimeout(() => setCopiedId(null), 1500);
  };

  const handleInject = async (entry: HistoryEntry) => {
    onInjectText(displayText(entry));
    setInjectedId(entry.id);
    setTimeout(() => setInjectedId(null), 1500);
  };

  const undoAi = (entry: HistoryEntry) => {
    // Server already restores clipboard via the inject_text path; don't double-write.
    onInjectText(undoAiText(entry));
    setCopiedId(entry.id);
    setInjectedId(entry.id);
    setTimeout(() => {
      setCopiedId(null);
      setInjectedId(null);
    }, 1500);
  };

  const clearAll = async () => {
    setClearing(true);
    try {
      await api.clearAllHistory();
      setEntries([]);
      setConfirmClear(false);
    } catch (e) {
      console.error("Failed to clear history:", e);
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
    <div className="max-w-2xl mx-auto px-7 py-8 animate-fade-rise">
      <header className="flex items-center justify-between mb-6">
        <div>
          <h1 className="font-display text-[22px] font-semibold tracking-tight text-ink">
            History
          </h1>
          <p className="text-[12.5px] text-muted mt-0.5">
            Stored locally on your device · 100% private
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
          onChange={(e) => setQuery(e.target.value)}
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
                            className="icon-btn hover:bg-base-2"
                            title="Insert into active application"
                            aria-label="Insert into active app"
                            onClick={() => handleInject(entry)}
                          >
                            {injectedId === entry.id ? (
                              <Check className="w-3.5 h-3.5 text-emerald-600 dark:text-emerald-400" />
                            ) : (
                              <CornerDownLeft className="w-3.5 h-3.5 text-muted" />
                            )}
                          </button>
                          {edited && (
                            <button
                              className="icon-btn hover:bg-base-2"
                              title="Undo AI edit"
                              aria-label="Undo AI edit"
                              onClick={() => undoAi(entry)}
                            >
                              <Undo2 className="w-3.5 h-3.5 text-muted" />
                            </button>
                          )}
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
    </div>
  );
};
