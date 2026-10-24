import type {
  IntelligenceDownloadEvent,
  PythonSetupProgress,
  RuntimeDownloadEvent,
} from "../types";

const LABELS: Record<string, string> = {
  checking_runtime: "Checking runtime",
  installing_runtime: "Installing runtime",
  starting: "Preparing model download",
  downloading: "Downloading model",
  verifying: "Verifying model files",
  committing: "Activating model",
  cancelling: "Cancelling setup",
  complete: "Model ready",
  cancelled: "Setup cancelled",
  error: "Setup failed",
};
export function writingSetupLabel(phase: string): string {
  return LABELS[phase] ?? "Checking setup status";
}
export function SetupProgress({
  writing,
  runtime,
  speech,
  speechModelPending = false,
  onReview,
}: {
  writing?: IntelligenceDownloadEvent | null;
  runtime?: RuntimeDownloadEvent | null;
  speech?: PythonSetupProgress | null;
  speechModelPending?: boolean;
  onReview: () => void;
}) {
  const writingRunning = writing && !["complete", "cancelled", "error"].includes(writing.phase);
  const runtimeRunning = runtime && !["complete", "cancelled", "error"].includes(runtime.phase);
  const speechActive =
    speech &&
    !["complete", "cancelled"].includes(speech.phase) &&
    (speech.phase !== "error" || (!writingRunning && !runtimeRunning));
  const writingActive =
    writing &&
    !["complete", "cancelled"].includes(writing.phase) &&
    (writing.phase !== "error" || !runtimeRunning);
  const runtimeActive = runtime && !["complete", "cancelled"].includes(runtime.phase);
  if (!speechActive && !writingActive && !runtimeActive) return null;
  const runtimeFirst =
    writing?.phase === "checking_runtime" || writing?.phase === "installing_runtime";
  const phase = speechActive ? speech.phase : writingActive ? writing.phase : runtime?.phase;
  const failed = phase === "error";
  const title = speechActive
    ? "Speech runtime"
    : runtimeFirst || (!writingActive && runtimeActive)
      ? runtimeFirst
        ? "Writing runtime"
        : "Inference runtime"
      : "Writing model";
  const message = speechActive
    ? failed
      ? "Speech runtime setup failed"
      : `Preparing speech runtime · ${speech.phase.replace(/_/g, " ")}`
    : writingActive
      ? writingSetupLabel(writing.phase)
      : failed
        ? "Runtime setup failed"
        : "Installing and verifying runtime";
  const progress = speechActive
    ? speech.progress_pct
    : runtimeFirst || !writingActive
      ? runtime?.progress_pct
      : writing?.progress_pct;
  const error = speechActive ? speech.error : writingActive ? writing.error : runtime?.error;
  return (
    <div className="sidebar-card" role={failed ? "alert" : "status"}>
      <strong>{title}</strong>
      <p>{message}</p>
      {failed ? (
        <p>{error ?? "Open Models to retry setup."}</p>
      ) : (
        <>
          {typeof progress === "number" && (
            <>
              <div className="sidebar-meter" aria-hidden>
                <div style={{ width: `${Math.max(0, Math.min(100, progress))}%` }} />
              </div>
              <span className="tabular-nums text-muted">{Math.round(progress)}%</span>
            </>
          )}
          {(runtimeFirst || speechModelPending) && (
            <p>
              Model download follows runtime verification. You can close Settings while setup
              continues.
            </p>
          )}
        </>
      )}
      <button type="button" className="btn btn-secondary" onClick={onReview}>
        {failed ? "Review and retry" : "View setup"}
      </button>
    </div>
  );
}
