import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Overlay } from "./Overlay";
import { StreamingTranscriptPayload } from "../types";

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

describe("Overlay", () => {
  /// The core regression: these two phases used to render identically apart from
  /// one word, so a stalled dictation was indistinguishable from a working one.
  it("renders a different phase marker for transcribing and polishing", () => {
    const { container: transcribing } = render(
      <Overlay appState="PROCESSING" transcript={payload()} standalone />,
    );
    const { container: polishing } = render(
      <Overlay appState="PROCESSING" transcript={payload({ stage: "polishing" })} standalone />,
    );

    expect(transcribing.querySelector(".hud")?.getAttribute("data-phase")).toBe("transcribe");
    expect(polishing.querySelector(".hud")?.getAttribute("data-phase")).toBe("polish");
  });

  it("shows only the current status without pipeline details or transcript text", () => {
    const { container } = render(
      <Overlay
        appState="PROCESSING"
        transcript={payload({ full_text: "Private transcript", stage: "polishing" })}
        standalone
      />,
    );
    expect(screen.getByText("Polishing")).toBeInTheDocument();
    expect(screen.queryByText("Private transcript")).not.toBeInTheDocument();
    expect(container.querySelector(".hud-rail, .hud-mode, .hud-timing, .hud-text")).toBeNull();
  });

  it("shows a brief completion label instead of repeating the transcript", () => {
    render(
      <Overlay
        appState="READY"
        transcript={payload({ full_text: "Hello." })}
        standalone
        extraMessage="Inserted"
      />,
    );
    expect(screen.getByText("Done")).toBeInTheDocument();
    expect(screen.queryByText("Hello.")).not.toBeInTheDocument();
  });

  it("gives a concise recovery instruction on failure", () => {
    render(<Overlay appState="ERROR" transcript={payload()} standalone />);
    expect(screen.getByText("Couldn't finish · open Reflow")).toBeInTheDocument();
  });

  it("does not show an empty capsule in the standalone window", () => {
    const { container } = render(<Overlay appState="READY" transcript={payload()} standalone />);
    expect(container.firstChild).toBeNull();
  });

  /// The clipboard fallback is the message the user must actually act on, so it
  /// has to reach the HUD rather than being replaced by a generic "Inserted".
  it("surfaces the clipboard fallback message", () => {
    render(
      <Overlay
        appState="IDLE"
        transcript={payload({ full_text: "Hello." })}
        standalone
        extraMessage="Copied — press Ctrl+V"
      />,
    );
    expect(screen.getByText("Copied — press Ctrl+V")).toBeInTheDocument();
  });

  it("renders nothing when floating and there is nothing to report", () => {
    const { container } = render(<Overlay appState="READY" transcript={payload()} />);
    expect(container.firstChild).toBeNull();
  });

  it("announces stage changes to assistive technology", () => {
    const { container } = render(
      <Overlay appState="RECORDING" transcript={payload()} standalone />,
    );
    const hud = container.querySelector(".hud");
    expect(hud?.getAttribute("role")).toBe("status");
    expect(hud?.getAttribute("aria-live")).toBe("polite");
  });
});
