import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { DictionaryPage } from "./DictionaryPage";
import { api } from "../../services/tauriApi";

it("retains the draft when a dictionary save fails", async () => {
  const settings = await api.getSettings();
  render(
    <DictionaryPage settings={settings} onUpdateSettings={vi.fn().mockResolvedValue(false)} />,
  );
  fireEvent.change(screen.getByRole("textbox", { name: "New vocabulary term" }), {
    target: { value: "Reflow" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Add term" }));
  await screen.findByRole("alert");
  expect(screen.getByRole("textbox", { name: "New vocabulary term" })).toHaveValue("Reflow");
});

it("saves vocabulary through the authoritative settings update", async () => {
  const settings = await api.getSettings();
  const save = vi.fn().mockResolvedValue(true);
  render(<DictionaryPage settings={settings} onUpdateSettings={save} />);
  fireEvent.change(screen.getByRole("textbox", { name: "New vocabulary term" }), {
    target: { value: "Reflow" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Add term" }));
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith({
      dictionary_terms: expect.arrayContaining([
        expect.objectContaining({ term: "Reflow", preferred_spelling: "Reflow" }),
      ]),
    }),
  );
  await waitFor(() =>
    expect(screen.getByRole("textbox", { name: "New vocabulary term" })).toHaveValue(""),
  );
});
