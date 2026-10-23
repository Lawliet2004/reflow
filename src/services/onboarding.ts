import { AppSettings, MicrophoneHealth } from "../types";

const KEY = "reflow.onboarding.v1";
export type SetupOutcome = "in_progress" | "deferred" | "completed";
export interface SetupCheckpoint {
  version: 1;
  stage: number;
  outcome: SetupOutcome;
  microphone?: string;
  shortcut?: { configuration: string; mode: "shortcut" | "button" };
  speech?: string;
  delivery?: "inserted" | "clipboard";
}
export const microphoneConfiguration = (settings: AppSettings) =>
  JSON.stringify([settings.microphone_device_id, settings.input_gain]);
export const shortcutConfiguration = (settings: AppSettings) =>
  JSON.stringify([settings.hotkey, settings.push_to_talk]);
export const speechConfiguration = (settings: AppSettings) =>
  JSON.stringify([
    settings.asr.model,
    settings.asr.runtime,
    settings.asr.device,
    settings.asr.precision,
    settings.asr.python_path,
    settings.language,
  ]);
export const dependencyConfiguration = (settings: AppSettings) =>
  JSON.stringify([
    settings.asr.model,
    settings.asr.runtime,
    settings.asr.device,
    settings.asr.python_path,
  ]);
export const trialConfiguration = (settings: AppSettings) =>
  `${microphoneConfiguration(settings)}:${speechConfiguration(settings)}`;
export const usableMicrophone = (health: MicrophoneHealth) =>
  health.duration_ms > 0 &&
  health.peak >= 0.001 &&
  health.rms >= 0.005 &&
  health.clipped_pct <= 0.5 &&
  health.dropped_chunks === 0;

export function readSetupCheckpoint(): SetupCheckpoint {
  try {
    const value: SetupCheckpoint = JSON.parse(localStorage.getItem(KEY) || "null");
    if (
      value?.version === 1 &&
      Number.isInteger(value.stage) &&
      value.stage >= 0 &&
      value.stage <= 5 &&
      ["in_progress", "deferred", "completed"].includes(value.outcome) &&
      (value.microphone === undefined || typeof value.microphone === "string") &&
      (value.speech === undefined || typeof value.speech === "string") &&
      (value.shortcut === undefined ||
        (typeof value.shortcut.configuration === "string" &&
          ["shortcut", "button"].includes(value.shortcut.mode)))
    )
      return value;
  } catch {
    /* Corrupt or unavailable storage starts at an honest input check. */
  }
  return { version: 1, stage: 0, outcome: "in_progress" };
}

export function writeSetupCheckpoint(value: SetupCheckpoint) {
  try {
    localStorage.setItem(KEY, JSON.stringify(value));
  } catch (failure) {
    console.error("Could not save setup progress:", failure);
  }
}

/** Older completion flags mean dismissed, since older versions also wrote them on deferral. */
export function setupDismissed() {
  const checkpoint = readSetupCheckpoint();
  try {
    if (localStorage.getItem(KEY)) return checkpoint.outcome !== "in_progress";
    return localStorage.getItem("reflow.onboarding.completed") === "1";
  } catch {
    return false;
  }
}

export function resumeSetupStage(value: SetupCheckpoint, settings: AppSettings) {
  if (value.microphone !== microphoneConfiguration(settings)) return 0;
  if (value.shortcut?.configuration !== shortcutConfiguration(settings))
    return Math.min(value.stage, 1);
  if (value.speech !== speechConfiguration(settings)) return Math.min(value.stage, 2);
  // Recheck runtime health after reopening, then consult live model status.
  return Math.min(value.stage, 3);
}
