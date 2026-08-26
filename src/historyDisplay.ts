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

export function hasAiEdit(entry: HistoryEntry): boolean {
  if (entry.rewriter_used === true) return true;
  return historyOriginal(entry).trim() !== historyCleaned(entry).trim();
}
