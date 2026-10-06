import { act, render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { DictateHome } from "./DictateHome";
import { api } from "../services/tauriApi";
import * as bridge from "../services/tauriApi";
import type { AppSettings } from "../types";

async function home(update = vi.fn(), overrides: Partial<AppSettings> = {}) {
  const settings = await api.getSettings();
  vi.spyOn(api, "getHistory").mockResolvedValue([]);
  return render(
    <DictateHome
      settings={{ ...settings, ...overrides }}
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
      onUpdateSettings={update}
      onOpenHistory={vi.fn()}
    />,
  );
}

describe("transcript actions", () => {
  it("refreshes recent dictations after a correction in another application", async () => {
    const listeners = new Map<string, (payload: unknown) => void>();
    const unsubscribe = vi.fn();
    vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
      listeners.set(event, handler);
      return unsubscribe;
    });
    const view = await home();
    await waitFor(() => expect(api.getHistory).toHaveBeenCalledTimes(1));
    vi.mocked(api.getHistory).mockResolvedValue([
      {
        id: "corrected",
        created_at: new Date().toISOString(),
        duration_ms: 100,
        language: "en",
        raw_transcript: "Use type script",
        smart_transcript: "Use type script",
        final_transcript: "Use TypeScript",
        application_name: "Chat",
        application_process: "chat.exe",
        word_count: 2,
        character_count: 14,
        model_version: "test",
        processing_mode: "smart",
      },
    ]);
    await act(async () => listeners.get("history:updated")?.({ id: "corrected" }));
    expect(await screen.findByText("Use TypeScript")).toBeInTheDocument();
    view.unmount();
    expect(unsubscribe).toHaveBeenCalled();
  });

  it("lets users choose either LLM or speech only without installing models", async () => {
    const update = vi.fn();
    const install = vi.spyOn(api, "installIntelligenceModel").mockResolvedValue();
    await home(update);
    const selector = screen.getByRole("combobox", { name: "LLM model" });
    fireEvent.change(selector, { target: { value: "qwen3.5-2b" } });
    expect(update).toHaveBeenLastCalledWith(
      expect.objectContaining({ intelligence_tier: "deep_context" }),
    );
    fireEvent.change(selector, { target: { value: "qwen3.5-0.8b" } });
    expect(update).toHaveBeenLastCalledWith(
      expect.objectContaining({ intelligence_tier: "smart_flow" }),
    );
    fireEvent.change(selector, { target: { value: "none" } });
    expect(update).toHaveBeenLastCalledWith(
      expect.objectContaining({ intelligence_tier: "raw_verbatim", flow_model: "none" }),
    );
    expect(install).not.toHaveBeenCalled();
  });

  it("keeps the chosen LLM when changing cleanup intensity", async () => {
    const update = vi.fn();
    await home(update, { intelligence_tier: "deep_context", cleanup_level: "high" });
    fireEvent.click(screen.getByRole("radio", { name: "Minimal" }));
    expect(update).toHaveBeenLastCalledWith({ cleanup_level: "light" });
  });

  it("shows writing style for light cleanup with an LLM and disables AI cleanup without one", async () => {
    const view = await home(vi.fn(), { intelligence_tier: "smart_flow", cleanup_level: "light" });
    expect(screen.getByRole("combobox", { name: "Writing style" })).toBeInTheDocument();
    view.unmount();
    await home(vi.fn(), { intelligence_tier: "raw_verbatim", cleanup_level: "light" });
    expect(screen.queryByRole("combobox", { name: "Writing style" })).not.toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Natural" })).toBeDisabled();
    expect(screen.getByRole("radio", { name: "Structured" })).toBeDisabled();
  });
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
    const clean = screen.getByRole("radio", { name: "Minimal" });
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
    expect(screen.getByRole("combobox", { name: "LLM model" })).toBeDisabled();
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
