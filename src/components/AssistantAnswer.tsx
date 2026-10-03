import React, { useEffect, useRef, useState } from "react";
import { api } from "../services/tauriApi";

export function assistantProposal(response: string): "search" | "note" | null {
  const line = response.trim();
  if ([...line].some((char) => char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127)) return null;
  const match = /^(SEARCH|NOTE): (.+)$/.exec(line);
  if (!match || !match[2].trim()) return null;
  const length = [...match[2].trim()].length;
  return length <= (match[1] === "SEARCH" ? 500 : 4000)
    ? match[1] === "SEARCH"
      ? "search"
      : "note"
    : null;
}

export const AssistantAnswer: React.FC<{ response: string; onDismiss: () => void }> = ({
  response,
  onDismiss,
}) => {
  const proposal = assistantProposal(response);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const [message, setMessage] = useState("");
  const dismiss = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    dismiss.current?.focus();
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onDismiss();
    };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, [onDismiss]);
  return (
    <div className="assistant-response" role="status" aria-live="polite">
      <div className="flex justify-between items-center gap-2">
        <strong>Reflow assistant</strong>
        <button ref={dismiss} aria-label="Dismiss answer" onClick={onDismiss}>
          ×
        </button>
      </div>
      <p className="assistant-answer-text whitespace-pre-wrap">
        {proposal
          ? response
              .trim()
              .slice(response.trim().indexOf(":") + 1)
              .trim()
          : response}
      </p>
      <div className="assistant-answer-actions">
        {proposal && (
          <button
            className="btn btn-primary"
            disabled={busy || done}
            onClick={async () => {
              setBusy(true);
              setMessage("");
              try {
                setMessage(await api.executeAssistantTool(response));
                setDone(true);
              } catch (e) {
                setMessage(String(e));
              } finally {
                setBusy(false);
              }
            }}
          >
            {busy ? "Working…" : done ? "Done" : proposal === "search" ? "Search web" : "Save note"}
          </button>
        )}
        {proposal === "search" && !done && (
          <p className="text-xs mt-1">Opens this query in your browser.</p>
        )}
        {message && <p className="text-xs mt-1">{message}</p>}
      </div>
    </div>
  );
};
