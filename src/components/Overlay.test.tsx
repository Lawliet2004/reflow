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

  it("reduces the completion to an icon instead of a text label", () => {
    const { container } = render(
      <Overlay
        appState="READY"
        transcript={payload({ full_text: "Hello." })}
        standalone
        extraMessage="Inserted"
      />,
    );
    expect(screen.getByText("Done")).toHaveClass("sr-only");
    expect(container.querySelector(".hud-label")).toBeNull();
    expect(screen.queryByText("Hello.")).not.toBeInTheDocument();
  });

  it("reduces a failure to an alert icon instead of recovery text", () => {
    const { container } = render(<Overlay appState="ERROR" transcript={payload()} standalone />);
    expect(screen.getByText("Couldn't finish · open Reflow")).toHaveClass("sr-only");
    expect(container.querySelector(".hud-label")).toBeNull();
    expect(container.querySelector(".hud-indicator")?.getAttribute("data-phase")).toBe("error");
  });

  it("does not show an empty capsule in the standalone window", () => {
    const { container } = render(<Overlay appState="READY" transcript={payload()} standalone />);
    expect(container.firstChild).toBeNull();
  });

  /// The clipboard fallback is announced to assistive technology and available
  /// on hover, but the pill itself stays wordless.
  it("keeps the clipboard fallback out of the visible pill", () => {
    const { container } = render(
      <Overlay
        appState="IDLE"
        transcript={payload({ full_text: "Hello." })}
        standalone
        extraMessage="Copied — press Ctrl+V"
      />,
    );
    expect(screen.getByText("Copied — press Ctrl+V")).toHaveClass("sr-only");
    expect(container.querySelector(".hud-label")).toBeNull();
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

  it("reacts to audio in the waveform pill and announces processing and completion", () => {
    const { container, rerender } = render(
      <Overlay appState="RECORDING" transcript={payload()} hudStyle="waveform" standalone />,
    );
    expect(screen.getByRole("status")).toHaveAttribute("data-style", "waveform");
    expect(screen.getByText("Listening")).toHaveClass("sr-only");
    const quietHeight = container.querySelector<HTMLElement>(".capsule-wave-bar")!.style.height;
    const wave = container.querySelector(".capsule-wave");
    rerender(
      <Overlay
        appState="RECORDING"
        transcript={payload({ audio_level: 0.3 })}
        hudStyle="waveform"
        standalone
      />,
    );
    expect(
      parseFloat(container.querySelector<HTMLElement>(".capsule-wave-bar")!.style.height),
    ).toBeGreaterThan(parseFloat(quietHeight));

    rerender(
      <Overlay appState="PROCESSING" transcript={payload()} hudStyle="waveform" standalone />,
    );
    expect(screen.getByText("Transcribing")).toHaveClass("sr-only");
    expect(container.querySelector(".capsule-wave")).toHaveAttribute("data-active", "false");

    rerender(
      <Overlay
        appState="READY"
        transcript={payload({ full_text: "Private transcript" })}
        extraMessage="Inserted"
        hudStyle="waveform"
        standalone
      />,
    );
    expect(screen.getByText("Done")).toHaveClass("sr-only");
    expect(container.querySelector(".capsule-check")).not.toBeNull();
    // Keep the same bars mounted so completion can crossfade without a blank frame.
    expect(container.querySelector(".capsule-wave")).toBe(wave);
    expect(screen.queryByText("Private transcript")).not.toBeInTheDocument();
  });

  it("keeps the waveform capsule stable when insertion finishes without polish", () => {
    const { container, rerender } = render(
      <Overlay appState="RECORDING" transcript={payload()} hudStyle="waveform" standalone />,
    );
    const wave = container.querySelector(".capsule-wave");
    rerender(
      <Overlay
        appState="READY"
        transcript={payload({ full_text: "Private transcript" })}
        extraMessage="Inserted (unpolished)"
        hudStyle="waveform"
        standalone
      />,
    );
    expect(screen.getByRole("status")).not.toHaveAttribute("data-message");
    expect(container.querySelector(".capsule-wave")).toBe(wave);
    expect(container.querySelector(".capsule-check")).not.toBeNull();
    expect(screen.getByText("Inserted (unpolished)")).toHaveClass("sr-only");
    expect(screen.getByRole("status")).toHaveAttribute("title", "Inserted (unpolished)");
  });

  it.each(["status", "waveform"] as const)(
    "silently hides a no-speech result in %s style",
    (hudStyle) => {
      const { container } = render(
        <Overlay
          appState="READY"
          transcript={payload()}
          extraMessage="No speech"
          hudStyle={hudStyle}
          standalone
        />,
      );
      expect(container.firstChild).toBeNull();
    },
  );

  it.each([
    ["IDLE", "Copied — press Ctrl+V", "Copied — press Ctrl+V", false],
    ["ERROR", "Failed", "Couldn't finish · open Reflow", true],
    ["READY", "Inserted (LLM error: timeout)", "Inserted (LLM error: timeout)", false],
    [
      "READY",
      "Inserted (unpolished) · Transcript incomplete",
      "Inserted (unpolished) · Transcript incomplete",
      false,
    ],
    ["READY", "No mic signal · USB Microphone", "No mic signal · USB Microphone", true],
  ] as const)(
    "keeps %s %s outcomes wordless in the waveform style",
    (appState, extraMessage, label, failed) => {
      const { container } = render(
        <Overlay
          appState={appState}
          transcript={payload()}
          extraMessage={extraMessage}
          hudStyle="waveform"
        />,
      );
      const hud = screen.getByRole("status");
      // The capsule never expands into a message: the outcome is announced and
      // available on hover only.
      expect(hud).not.toHaveAttribute("data-message");
      expect(container.querySelector(".hud-label")).toBeNull();
      expect(screen.getByText(label)).toHaveClass("sr-only");
      expect(hud.getAttribute("title")).toBe(label);
      expect(container.querySelector(".capsule-check")).not.toBeNull();
      expect(Boolean(container.querySelector(".capsule-alert"))).toBe(failed);
      expect(container.querySelector(".capsule-wave")).not.toBeNull();
    },
  );
});
