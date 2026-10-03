import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { DictateHome } from "./DictateHome";
import { api } from "../services/tauriApi";

async function home() {
  const settings = await api.getSettings();
  vi.spyOn(api, "getHistory").mockResolvedValue([]);
  return render(
    <DictateHome
      settings={settings}
      appState="READY"
      modelStatus={null}
      transcript={{
        full_text: "Hello world",
        committed_prefix: "",
        mutable_suffix: "",
        language: "en",
        audio_level: 0,
        stage: "",
      }}
      latencyMetrics={null}
      latencyPercentiles={null}
      onStartRecording={vi.fn()}
      onStopRecording={vi.fn()}
      onUpdateSettings={vi.fn()}
      onOpenHistory={vi.fn()}
    />,
  );
}

describe("transcript actions", () => {
  it("restores an edited transcript without changing the clipboard or another app", async () => {
    await home();
    fireEvent.change(screen.getByRole("textbox", { name: "Transcript" }), {
      target: { value: "Edited text" },
    });
    expect(screen.getByRole("textbox", { name: "Transcript" })).toHaveValue("Edited text");
    fireEvent.click(screen.getByRole("button", { name: "Undo transcript edit" }));
    expect(screen.getByRole("textbox", { name: "Transcript" })).toHaveValue("Hello world");
  });
  it("reports clipboard failures without showing success", async () => {
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText: vi.fn().mockRejectedValue(new Error("denied")) },
    });
    await home();
    fireEvent.click(screen.getByRole("button", { name: "Copy" }));
    await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("Could not copy"));
    expect(screen.queryByText("Copied")).not.toBeInTheDocument();
  });

  it("confirms copy on the button, supports Ctrl+Enter, and moves cleanup level with arrows", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: { writeText } });
    const view = await home();
    fireEvent.keyDown(screen.getByRole("textbox", { name: "Transcript" }), {
      key: "Enter",
      ctrlKey: true,
    });
    await waitFor(() => expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument());
    expect(writeText).toHaveBeenCalledWith("Hello world");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();

    const update = vi.fn();
    view.unmount();
    const settings = await api.getSettings();
    render(
      <DictateHome
        settings={{ ...settings, cleanup_level: "light" }}
        appState="READY"
        modelStatus={null}
        transcript={{
          full_text: "",
          committed_prefix: "",
          mutable_suffix: "",
          language: "en",
          audio_level: 0,
          stage: "",
        }}
        latencyMetrics={null}
        latencyPercentiles={null}
        onStartRecording={vi.fn()}
        onStopRecording={vi.fn()}
        onUpdateSettings={update}
        onOpenHistory={vi.fn()}
      />,
    );
    const clean = screen.getByRole("radio", { name: "Clean" });
    expect(clean).toHaveAttribute("aria-checked", "true");
    fireEvent.keyDown(clean, { key: "ArrowRight" });
    expect(update).toHaveBeenCalledWith(expect.objectContaining({ cleanup_level: "medium" }));
  });

  it("blocks recording and transcript edits until insertion finishes", async () => {
    const settings = await api.getSettings();
    const modelStatus = {
      ...(await api.getModelStatus()),
      installed: true,
      loaded: true,
      is_loading: false,
      backend: "CPU",
    };
    render(
      <DictateHome
        settings={settings}
        appState="INJECTING"
        modelStatus={modelStatus}
        transcript={{
          full_text: "Hello",
          committed_prefix: "",
          mutable_suffix: "",
          language: "en",
          audio_level: 0,
          stage: "",
        }}
        latencyMetrics={null}
        latencyPercentiles={null}
        onStartRecording={vi.fn()}
        onStopRecording={vi.fn()}
        onUpdateSettings={vi.fn()}
        onOpenHistory={vi.fn()}
      />,
    );
    expect(screen.getByRole("button", { name: "Start recording" })).toBeDisabled();
    expect(screen.getByRole("textbox", { name: "Transcript" })).toHaveAttribute("readonly");
    expect(screen.getByText("Inserting your words")).toBeInTheDocument();
  });

  it("does not carry edits into a later identical dictation started by the hotkey", async () => {
    const settings = await api.getSettings();
    const props = {
      settings,
      modelStatus: null,
      transcript: {
        full_text: "Hello world",
        committed_prefix: "",
        mutable_suffix: "",
        language: "en",
        audio_level: 0,
        stage: "",
      },
      latencyMetrics: null,
      latencyPercentiles: null,
      onStartRecording: vi.fn(),
      onStopRecording: vi.fn(),
      onUpdateSettings: vi.fn(),
      onOpenHistory: vi.fn(),
    };
    const view = render(<DictateHome {...props} appState="READY" />);
    fireEvent.change(screen.getByRole("textbox", { name: "Transcript" }), {
      target: { value: "Edited words" },
    });
    view.rerender(
      <DictateHome
        {...props}
        appState="RECORDING"
        transcript={{ ...props.transcript, full_text: "" }}
      />,
    );
    view.rerender(<DictateHome {...props} appState="READY" />);
    expect(screen.getByRole("textbox", { name: "Transcript" })).toHaveValue("Hello world");
  });
});
