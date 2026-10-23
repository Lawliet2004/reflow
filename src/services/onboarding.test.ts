import { beforeEach, expect, it, vi } from "vitest";
import { api } from "./tauriApi";
import {
  microphoneConfiguration,
  readSetupCheckpoint,
  resumeSetupStage,
  setupDismissed,
  shortcutConfiguration,
  speechConfiguration,
  usableMicrophone,
  writeSetupCheckpoint,
} from "./onboarding";

beforeEach(() => {
  localStorage.clear();
});
it("recovers corrupt and future checkpoints without inventing completed setup", () => {
  for (const value of [
    "broken",
    '{"version":2,"stage":4}',
    '{"version":1,"stage":99,"outcome":"completed"}',
    '{"version":1,"stage":4,"outcome":"completed","shortcut":null}',
  ]) {
    localStorage.setItem("reflow.onboarding.v1", value);
    expect(readSetupCheckpoint()).toEqual({ version: 1, stage: 0, outcome: "in_progress" });
    expect(setupDismissed()).toBe(false);
  }
});
it("treats a legacy completion flag as dismissal and gives a new checkpoint priority", () => {
  localStorage.setItem("reflow.onboarding.completed", "1");
  expect(setupDismissed()).toBe(true);
  writeSetupCheckpoint({ version: 1, stage: 2, outcome: "in_progress" });
  expect(setupDismissed()).toBe(false);
  writeSetupCheckpoint({ version: 1, stage: 2, outcome: "deferred" });
  expect(setupDismissed()).toBe(true);
});
it("invalidates only the affected input stages and always rechecks runtime after restart", async () => {
  const settings = await api.getSettings();
  const checkpoint = {
    version: 1 as const,
    stage: 5,
    outcome: "deferred" as const,
    microphone: microphoneConfiguration(settings),
    shortcut: { configuration: shortcutConfiguration(settings), mode: "button" as const },
    speech: speechConfiguration(settings),
  };
  expect(resumeSetupStage(checkpoint, settings)).toBe(3);
  expect(resumeSetupStage(checkpoint, { ...settings, input_gain: settings.input_gain + 1 })).toBe(
    0,
  );
  expect(resumeSetupStage(checkpoint, { ...settings, hotkey: "Alt+K" })).toBe(1);
  expect(resumeSetupStage(checkpoint, { ...settings, language: "hi" })).toBe(2);
});
it("does not accept silent, clipped or dropped input as a healthy microphone", () => {
  const sample = {
    device: "mic",
    duration_ms: 3000,
    peak: 0.5,
    rms: 0.1,
    clipped_pct: 0,
    dropped_chunks: 0,
    assessment: "",
  };
  expect(usableMicrophone(sample)).toBe(true);
  for (const invalid of [
    { duration_ms: 0 },
    { peak: 0 },
    { rms: 0.001 },
    { clipped_pct: 1 },
    { dropped_chunks: 1 },
    { rms: NaN },
  ]) {
    expect(usableMicrophone({ ...sample, ...invalid })).toBe(false);
  }
});
it("continues with a recoverable checkpoint when storage is denied", () => {
  vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
    throw new Error("denied");
  });
  expect(readSetupCheckpoint().outcome).toBe("in_progress");
  expect(setupDismissed()).toBe(false);
});
