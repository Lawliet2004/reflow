import React, { Profiler } from "react";
import { writeFileSync } from "node:fs";
import { render, screen, fireEvent, act } from "@testing-library/react";
import { it, vi, expect } from "vitest";
import { App } from "./App";
import * as bridge from "./services/tauriApi";

it("measures whole-window commits from microphone levels away from dictation", async () => {
  const model = await bridge.api.getModelStatus();
  vi.spyOn(bridge.api, "getModelStatus").mockResolvedValue({
    ...model,
    installed: true,
    loaded: true,
    is_loading: false,
    is_downloading: false,
  });
  vi.spyOn(bridge, "isTauri").mockReturnValue(true);
  const handlers = new Map<string, (payload: unknown) => void>();
  vi.spyOn(bridge, "safeListen").mockImplementation(async (event, handler) => {
    handlers.set(event, handler as (payload: unknown) => void);
    return () => {};
  });
  let commits = 0;
  render(
    <Profiler id="app" onRender={() => commits++}>
      <App />
    </Profiler>,
  );
  await screen.findByRole("heading", { name: /Speak freely/ });
  await act(async () => {});
  fireEvent.click(screen.getByRole("button", { name: "History" }));
  await act(async () => {});
  await act(async () => handlers.get("app:state-changed")?.("RECORDING"));
  const before = commits;
  const start = performance.now();
  for (let i = 0; i < 30; i++) {
    await act(async () => handlers.get("recording:audio-level")?.((i % 10) / 20));
  }
  const measurement = {
    workload: "30 microphone level events while History is open",
    window_commits: commits - before,
    elapsed_ms: performance.now() - start,
  };
  if (process.env.REFLOW_UI_PROFILE_OUTPUT) {
    writeFileSync(process.env.REFLOW_UI_PROFILE_OUTPUT, JSON.stringify(measurement));
  }
  expect(handlers.has("recording:audio-level")).toBe(true);
  expect(commits - before).toBe(0);
  fireEvent.click(screen.getByRole("button", { name: "Home" }));
  await screen.findByRole("button", { name: "Stop recording" });
  await act(async () =>
    handlers.get("transcript:final")?.({
      committed_prefix: "",
      mutable_suffix: "",
      full_text: "Retained transcript",
      language: "en",
      audio_level: 0,
      stage: "done",
    }),
  );
  await act(async () => handlers.get("app:state-changed")?.("READY"));
  expect(await screen.findByRole("textbox", { name: "Transcript" })).toHaveValue(
    "Retained transcript",
  );
  fireEvent.click(screen.getByRole("button", { name: "History" }));
  await act(async () => handlers.get("app:state-changed")?.("RECORDING"));
  fireEvent.click(screen.getByRole("button", { name: "Home" }));
  await act(async () => handlers.get("app:state-changed")?.("READY"));
  expect(screen.queryByDisplayValue("Retained transcript")).not.toBeInTheDocument();
});
