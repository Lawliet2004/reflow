import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import { AssistantAnswer, assistantProposal } from "./AssistantAnswer";
import { api } from "../services/tauriApi";

describe("assistant proposals", () => {
  it("focuses dismissal and accepts Escape from the keyboard", () => {
    const dismiss = vi.fn();
    render(<AssistantAnswer response="A local answer" onDismiss={dismiss} />);
    expect(screen.getByRole("button", { name: "Dismiss answer" })).toHaveFocus();
    fireEvent.keyDown(screen.getByRole("button", { name: "Dismiss answer" }), { key: "Escape" });
    expect(dismiss).toHaveBeenCalledOnce();
  });
  it("never activates a tool on receipt, and allows one explicit activation", async () => {
    const execute = vi.spyOn(api, "executeAssistantTool").mockResolvedValue("Saved in Notes");
    render(<AssistantAnswer response="NOTE: Remember tea" onDismiss={vi.fn()} />);
    expect(execute).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Save note" }));
    await waitFor(() => expect(screen.getByText("Saved in Notes")).toBeInTheDocument());
    expect(execute).toHaveBeenCalledOnce();
    expect(execute).toHaveBeenCalledWith("NOTE: Remember tea");
    expect(screen.getByRole("button", { name: "Done" })).toBeDisabled();
  });
  it("keeps a failed proposal available for retry", async () => {
    vi.spyOn(api, "executeAssistantTool").mockRejectedValue(new Error("History key unavailable"));
    render(<AssistantAnswer response="NOTE: Tea" onDismiss={vi.fn()} />);
    fireEvent.click(screen.getByRole("button", { name: "Save note" }));
    await waitFor(() => expect(screen.getByText(/History key unavailable/)).toBeInTheDocument());
    expect(screen.getByRole("button", { name: "Save note" })).toBeEnabled();
  });
  it("rejects wrappers, multiple commands, shell commands and oversized queries", () => {
    for (const response of [
      "RUN: echo hi",
      "SEARCH: one\nNOTE: two",
      "SEARCH:",
      "Here is NOTE: text",
      `SEARCH: ${"a".repeat(501)}`,
    ]) {
      expect(assistantProposal(response)).toBeNull();
    }
    expect(assistantProposal("SEARCH: Rust borrow checker")).toBe("search");
  });
});
