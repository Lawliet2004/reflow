import { describe, expect, it } from "vitest";
import { StreamingTranscriptPayload } from "../../types";
import { derivePhase, isNeutralOutcome, phaseLabel, railStages, stageStatus } from "./stages";

function payload(over: Partial<StreamingTranscriptPayload> = {}): StreamingTranscriptPayload {
  return {
    committed_prefix: "",
    mutable_suffix: "",
    full_text: "",
    language: "en",
    audio_level: 0,
    stage: "",
    ...over,
  };
}

describe("derivePhase", () => {
  it("maps recording to the listening stage", () => {
    expect(derivePhase("RECORDING", payload())).toBe("listen");
  });

  /// The distinction the old HUD could not express: transcribing and polishing
  /// rendered as the same pill with a different word.
  it("separates transcribing from polishing", () => {
    expect(derivePhase("PROCESSING", payload())).toBe("transcribe");
    expect(derivePhase("PROCESSING", payload({ stage: "polishing" }))).toBe("polish");
  });

  it("treats a polishing hint in the injection message as polishing", () => {
    expect(derivePhase("PROCESSING", payload(), "Polishing")).toBe("polish");
  });

  it("maps injecting to the insert stage", () => {
    expect(derivePhase("INJECTING", payload())).toBe("insert");
  });

  it("reports errors regardless of transcript contents", () => {
    expect(derivePhase("ERROR", payload({ full_text: "anything" }))).toBe("error");
  });

  /// A resting state with nothing to report must render nothing, or the overlay
  /// shows an empty capsule after every dictation.
  it("is hidden when at rest with nothing to show", () => {
    expect(derivePhase("READY", payload())).toBe("hidden");
    expect(derivePhase("IDLE", payload())).toBe("hidden");
  });

  it("is done when at rest with a result or a message", () => {
    expect(derivePhase("READY", payload({ full_text: "Hello." }))).toBe("done");
    expect(derivePhase("IDLE", payload(), "Copied — press Ctrl+V")).toBe("done");
  });
});

describe("railStages", () => {
  /// The rail is also how the user sees which mode they are in: no polish step
  /// means no LLM pass.
  it("omits the polish step when polishing is off", () => {
    expect(railStages(true)).toEqual(["listen", "transcribe", "polish", "insert"]);
    expect(railStages(false)).toEqual(["listen", "transcribe", "insert"]);
  });
});

describe("stageStatus", () => {
  it("marks passed stages done, the current one active and the rest pending", () => {
    expect(stageStatus("listen", "polish", true)).toBe("done");
    expect(stageStatus("transcribe", "polish", true)).toBe("done");
    expect(stageStatus("polish", "polish", true)).toBe("active");
    expect(stageStatus("insert", "polish", true)).toBe("pending");
  });

  it("completes the whole rail when the dictation is done", () => {
    for (const stage of railStages(true)) {
      expect(stageStatus(stage, "done", true)).toBe("done");
    }
  });

  /// With polishing off, reaching `insert` must still show the earlier steps as
  /// complete even though the rail is shorter.
  it("handles the shortened rail", () => {
    expect(stageStatus("listen", "insert", false)).toBe("done");
    expect(stageStatus("transcribe", "insert", false)).toBe("done");
    expect(stageStatus("insert", "insert", false)).toBe("active");
  });
});

describe("phaseLabel", () => {
  it("names each working stage", () => {
    expect(phaseLabel("listen", "", "")).toBe("Listening");
    expect(phaseLabel("transcribe", "", "")).toBe("Transcribing");
    expect(phaseLabel("polish", "", "")).toBe("Polishing");
    expect(phaseLabel("insert", "", "")).toBe("Inserting");
  });

  /// The injection message carries outcomes the phase cannot express — most
  /// importantly that the paste could not be delivered and the text is on the
  /// clipboard instead.
  it("prefers the injection message when done", () => {
    expect(phaseLabel("done", "Copied — press Ctrl+V", "Hello.")).toBe("Copied — press Ctrl+V");
    expect(phaseLabel("done", "", "Hello.")).toBe("Inserted");
    expect(phaseLabel("done", "", "")).toBe("No speech detected");
    expect(phaseLabel("done", "No speech", "")).toBe("No speech detected");
  });

  it("falls back to a readable error", () => {
    expect(phaseLabel("error", "", "")).toBe("Something went wrong");
    expect(phaseLabel("error", "Model is not loaded.", "")).toBe("Model is not loaded.");
  });
});

describe("isNeutralOutcome", () => {
  it("treats non-success outcomes as neutral so they are not styled as success", () => {
    expect(isNeutralOutcome("No speech detected")).toBe(true);
    expect(isNeutralOutcome("Copied — press Ctrl+V")).toBe(true);
    expect(isNeutralOutcome("No mic input")).toBe(true);
    expect(isNeutralOutcome("Inserted")).toBe(false);
  });
});
