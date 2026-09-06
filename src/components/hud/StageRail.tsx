import React from "react";
import { HudPhase, PipelineStage, STAGE_SHORT_LABELS, railStages, stageStatus } from "./stages";

interface StageRailProps {
  phase: HudPhase;
  polishEnabled: boolean;
  /** Compact rails drop the text labels and keep only the track. */
  compact?: boolean;
}

/**
 * The pipeline as a progress track: Listen -> Transcribe -> Polish -> Insert.
 *
 * This is the part that was missing. Every stage used to render the same pill
 * with a spinner and one word, so the HUD could not answer the only question a
 * user actually has while waiting — "is it still going, and how far?" A rail
 * answers it at a glance and, because the `polish` step disappears when the LLM
 * is off, it also shows which mode you are in without reading anything.
 */
export const StageRail: React.FC<StageRailProps> = ({ phase, polishEnabled, compact = false }) => {
  const stages = railStages(polishEnabled);

  return (
    <ol className="hud-rail" aria-label="Dictation progress">
      {stages.map((stage: PipelineStage, index) => {
        const status = stageStatus(stage, phase, polishEnabled);
        return (
          <li
            key={stage}
            className="hud-rail-step"
            data-status={status}
            data-stage={stage}
            aria-current={status === "active" ? "step" : undefined}
          >
            <span className="hud-rail-dot" aria-hidden="true" />
            {!compact && <span className="hud-rail-label">{STAGE_SHORT_LABELS[stage]}</span>}
            {index < stages.length - 1 && <span className="hud-rail-link" aria-hidden="true" />}
            <span className="sr-only">
              {STAGE_SHORT_LABELS[stage]}: {status}
            </span>
          </li>
        );
      })}
    </ol>
  );
};
