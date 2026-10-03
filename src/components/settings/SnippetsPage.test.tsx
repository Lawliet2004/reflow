import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { api } from "../../services/tauriApi";
import { SnippetsPage } from "./SnippetsPage";

describe("Snippets", () => {
  it("saves exact expansion text and closes only after a successful save", async () => {
    const settings = await api.getSettings();
    const save = vi.fn().mockResolvedValue(true);
    render(<SnippetsPage settings={{ ...settings, snippets: [] }} onUpdateSettings={save} />);
    fireEvent.click(screen.getByRole("button", { name: "Add snippet" }));
    fireEvent.change(screen.getByRole("textbox", { name: "Spoken trigger" }), {
      target: { value: "insert my address" },
    });
    fireEvent.change(screen.getByRole("textbox", { name: "Expansion" }), {
      target: { value: "42 avenue\nFRANCE" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save snippet" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith({
        snippets: [
          expect.objectContaining({
            trigger: "insert my address",
            expansion: "42 avenue\nFRANCE",
            enabled: true,
          }),
        ],
      }),
    );
    await waitFor(() =>
      expect(screen.queryByRole("textbox", { name: "Expansion" })).not.toBeInTheDocument(),
    );
  });
  it("keeps the draft when persistence fails", async () => {
    const settings = await api.getSettings();
    render(
      <SnippetsPage
        settings={{
          ...settings,
          snippets: [{ id: "a", trigger: "address", expansion: "Stored text", enabled: true }],
        }}
        onUpdateSettings={vi.fn().mockResolvedValue(false)}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    fireEvent.click(screen.getByRole("button", { name: "Save snippet" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("Could not save");
    expect(screen.getByRole("textbox", { name: "Expansion" })).toHaveValue("Stored text");
  });
});
