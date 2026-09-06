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

  it("marks the active rail step for the current phase", () => {
    const { container } = render(
      <Overlay appState="PROCESSING" transcript={payload({ stage: "polishing" })} standalone />,
    );

    const active = container.querySelector('.hud-rail-step[data-status="active"]');
    expect(active?.getAttribute("data-stage")).toBe("polish");
    // Earlier steps are complete.
    expect(
      container.querySelector('.hud-rail-step[data-stage="listen"]')?.getAttribute("data-status"),
    ).toBe("done");
  });

  /// The rail length is how the user sees which mode is active without reading.
  it("drops the polish step from the rail when polishing is off", () => {
    const { container } = render(
      <Overlay appState="RECORDING" transcript={payload()} standalone polishEnabled={false} />,
    );

    expect(container.querySelectorAll(".hud-rail-step")).toHaveLength(3);
    expect(container.querySelector('.hud-rail-step[data-stage="polish"]')).toBeNull();
  });

  it("always shows which mode the dictation is running in", () => {
    render(<Overlay appState="RECORDING" transcript={payload()} standalone polishEnabled />);
    expect(screen.getByText("Polished")).toBeInTheDocument();

    render(
      <Overlay appState="RECORDING" transcript={payload()} standalone polishEnabled={false} />,
    );
    expect(screen.getByText("Fast")).toBeInTheDocument();
  });

  it("shows live partial text while transcribing", () => {
    render(
      <Overlay
        appState="PROCESSING"
        transcript={payload({ committed_prefix: "hello ", mutable_suffix: "world" })}
        standalone
      />,
    );
    expect(screen.getByText("hello world")).toBeInTheDocument();
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

  it("hides per-stage timings unless developer mode is on", () => {
    const { container: off } = render(
      <Overlay appState="PROCESSING" transcript={payload()} standalone />,
    );
    expect(off.querySelector(".hud-timing")).toBeNull();

    const { container: on } = render(
      <Overlay appState="PROCESSING" transcript={payload()} standalone showTimings />,
    );
    expect(on.querySelector(".hud-timing")).not.toBeNull();
  });
});
