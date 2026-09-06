import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { HistoryView } from "./HistoryView";
import { api } from "../services/tauriApi";
import type { HistoryEntry } from "../types";

function entry(): HistoryEntry {
  const date = new Date();
  date.setDate(date.getDate() - 2);
  date.setHours(12, 0, 0, 0);
  return {
    id: "one",
    created_at: date.toISOString(),
    final_transcript: "Keep this thought",
    raw_transcript: "Keep this thought",
    smart_transcript: "",
    rewriter_used: false,
    duration_ms: 1000,
    language: "en",
    application_name: "",
    application_process: "",
    word_count: 3,
    character_count: 17,
    model_version: "test",
    processing_mode: "raw",
  };
}
it("includes the day before yesterday in Earlier", async () => {
  vi.spyOn(api, "getHistory").mockResolvedValue([entry()]);
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "Earlier" }));
  expect(screen.getByText("Keep this thought")).toBeInTheDocument();
});
it("keeps a transcript visible when deletion fails", async () => {
  vi.spyOn(api, "getHistory").mockResolvedValue([entry()]);
  vi.spyOn(api, "deleteHistoryItem").mockRejectedValue(new Error("database locked"));
  render(<HistoryView />);
  await screen.findByText("Keep this thought");
  fireEvent.click(screen.getByRole("button", { name: "Delete entry" }));
  await waitFor(() => expect(screen.getByRole("alert")).toHaveTextContent("Could not delete"));
  expect(screen.getByText("Keep this thought")).toBeInTheDocument();
});
