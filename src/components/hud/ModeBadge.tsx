import React from "react";
import { Sparkles, Zap } from "lucide-react";

interface ModeBadgeProps {
  polishEnabled: boolean;
  /** Model actually doing the polishing, shown as a tooltip. */
  modelLabel?: string;
}

/**
 * Which mode this dictation is running in.
 *
 * Always present, in every phase. The Fast/Polished switch is meant to be
 * flipped mid-workflow — from the tray, without opening Settings — so the HUD
 * has to make the current mode unambiguous. Otherwise the only way to tell
 * whether polishing ran is to look at the result and guess, and a missing badge
 * turns "why is this slower?" and "why wasn't this cleaned up?" into the same
 * unanswerable question.
 */
export const ModeBadge: React.FC<ModeBadgeProps> = ({ polishEnabled, modelLabel }) => {
  const label = polishEnabled ? "Polished" : "Fast";
  const title = polishEnabled
    ? `Polishing with ${modelLabel ?? "the local LLM"}`
    : "Transcription only — no LLM pass";

  return (
    <span className="hud-mode" data-mode={polishEnabled ? "polished" : "fast"} title={title}>
      {polishEnabled ? (
        <Sparkles className="w-3 h-3" aria-hidden="true" />
      ) : (
        <Zap className="w-3 h-3" aria-hidden="true" />
      )}
      <span>{label}</span>
    </span>
  );
};
