import { AppState, StreamingTranscriptPayload } from "../../types";

/**
 * A step in the dictation pipeline, as the user experiences it.
 *
 * The backend's `DoryEvent::Stage` is forwarded over `pipeline:stage` and is
 * authoritative when present; `app:state-changed` plus the `stage` field on
 * the streaming payload remain as the fallback for older backends and the web
 * preview. Either way the HUD can tell the four steps apart — the old HUD
 * rendered "Transcribing" and "Polishing" as visually identical 56px pills.
 */
export type PipelineStage = "listen" | "transcribe" | "polish" | "insert";

export type HudPhase = PipelineStage | "done" | "error" | "hidden";

/** Ordered rail steps. `polish` is absent when the LLM is switched off. */
export function railStages(polishEnabled: boolean): PipelineStage[] {
  return polishEnabled
    ? ["listen", "transcribe", "polish", "insert"]
    : ["listen", "transcribe", "insert"];
}

export const STAGE_LABELS: Record<PipelineStage, string> = {
  listen: "Listening",
  transcribe: "Transcribing",
  polish: "Polishing",
  insert: "Inserting",
};

export const STAGE_SHORT_LABELS: Record<PipelineStage, string> = {
  listen: "Listen",
  transcribe: "Transcribe",
  polish: "Polish",
  insert: "Insert",
};

/**
 * Backend pipeline stage, forwarded over `pipeline:stage` from
 * `DoryEvent::Stage`. When present it is authoritative; the
 * `transcript.stage` string-match below is only the fallback for older
 * backends and the web preview.
 */
export type BackendStage =
  "capture" | "resample" | "vad" | "asr" | "format" | "inject" | "history" | "api";

function phaseFromBackendStage(stage: BackendStage): PipelineStage {
  switch (stage) {
    case "inject":
    case "history":
    case "api":
      return "insert";
    default:
      return "transcribe";
  }
}

function isPolishing(
  transcript: StreamingTranscriptPayload,
  extraMessage?: string | null,
): boolean {
  const stage = (transcript.stage ?? "").toLowerCase();
  const extra = (extraMessage ?? "").toLowerCase();
  return stage === "polishing" || extra === "polishing" || extra.includes("polish");
}

/**
 * Which phase the HUD is in.
 *
 * `hidden` means there is nothing to show — the overlay window is only ever
 * visible during a dictation or briefly after one, so a resting state with no
 * message must render nothing rather than an empty capsule.
 */
export function derivePhase(
  appState: AppState,
  transcript: StreamingTranscriptPayload,
  extraMessage?: string | null,
  backendStage?: BackendStage | null,
): HudPhase {
  if (appState === "ERROR") return "error";
  if (appState === "RECORDING") return "listen";
  if (appState === "PROCESSING") {
    if (backendStage) {
      const mapped = phaseFromBackendStage(backendStage);
      if (mapped === "insert") return "transcribe";
      return isPolishing(transcript, extraMessage) ? "polish" : mapped;
    }
    return isPolishing(transcript, extraMessage) ? "polish" : "transcribe";
  }
  if (appState === "INJECTING") return "insert";
  const hasSomethingToShow =
    Boolean(transcript.full_text.trim()) || Boolean((extraMessage ?? "").trim());
  return hasSomethingToShow ? "done" : "hidden";
}

/**
 * How far along the rail we are: `done` for steps already passed, `active` for
 * the current one, `pending` for the rest.
 *
 * A phase that is not on the rail (because polish is off, or because we are in
 * `done`/`error`) still has to place every step sensibly, so `done` marks the
 * whole rail complete and `error` leaves it where it stopped.
 */
export function stageStatus(
  stage: PipelineStage,
  phase: HudPhase,
  polishEnabled: boolean,
): "done" | "active" | "pending" {
  if (phase === "done") return "done";
  const rail = railStages(polishEnabled);
  const currentIndex = rail.indexOf(phase as PipelineStage);
  const stageIndex = rail.indexOf(stage);
  if (currentIndex < 0 || stageIndex < 0) return "pending";
  if (stageIndex < currentIndex) return "done";
  if (stageIndex === currentIndex) return "active";
  return "pending";
}

/**
 * The headline for the current phase.
 *
 * `doneMessage` is whatever the injection feedback said, because that carries
 * the cases the phase alone cannot express — "Copied — press Ctrl+V" when the
 * paste could not be delivered, or a warning that the dictation was truncated.
 */
export function phaseLabel(phase: HudPhase, doneMessage: string, fullText: string): string {
  switch (phase) {
    case "listen":
    case "transcribe":
    case "polish":
    case "insert":
      return STAGE_LABELS[phase];
    case "error":
      return doneMessage.trim() || "Something went wrong";
    case "done": {
      const message = doneMessage.trim();
      if (message === "No speech" || message === "No speech detected") return "No speech detected";
      if (message) return message;
      return fullText.trim() ? "Inserted" : "No speech detected";
    }
    default:
      return "";
  }
}

/** `true` when the label reports a non-success outcome rather than progress. */
export function isNeutralOutcome(label: string): boolean {
  return (
    label === "No speech detected" ||
    label.startsWith("No mic") ||
    label.startsWith("Copied") ||
    label === "Something went wrong"
  );
}
