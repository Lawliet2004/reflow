import { HistoryEntry } from "./types";

/** Always the ASR verbatim. Used by Show original. */
export function historyOriginal(entry: HistoryEntry): string {
  return entry.raw_transcript ?? "";
}

export function historyCleaned(entry: HistoryEntry): string {
  return entry.final_transcript ?? "";
}

/**
 * Undo AI: restore pre-LLM Light text when a rewriter ran, otherwise the raw ASR.
 * Never returns smart when smart === final on Light (that hid the raw transcript).
 */
export function undoAiText(entry: HistoryEntry): string {
  if (entry.rewriter_used) {
    const smart = (entry.smart_transcript ?? "").trim();
    const finalText = (entry.final_transcript ?? "").trim();
    if (smart && smart !== finalText) {
      return entry.smart_transcript as string;
    }
  }
  return historyOriginal(entry);
}

const UNITS: [Intl.RelativeTimeFormatUnit, number][] = [
  ["minute", 60],
  ["hour", 24],
  ["day", 7],
];

/** "just now", "5 min. ago", "yesterday", "3 days ago"; older falls back to "Oct 2". */
export function relativeTime(iso: string, now = Date.now()): string {
  const then = new Date(iso).getTime();
  let delta = Math.round((then - now) / 60_000);
  if (Math.abs(delta) < 1) return "just now";
  const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto", style: "short" });
  for (const [unit, size] of UNITS) {
    if (Math.abs(delta) < size) return rtf.format(delta, unit);
    delta = Math.round(delta / size);
  }
  return new Date(iso).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

export function hasAiEdit(entry: HistoryEntry): boolean {
  if (entry.rewriter_used === true) return true;
  return historyOriginal(entry).trim() !== historyCleaned(entry).trim();
}
