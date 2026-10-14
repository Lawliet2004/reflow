import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { api } from "../../services/tauriApi";
import { OutputPage } from "./OutputPage";

it("preserves automatic paste by default and allows the terminal chord", async () => {
  const settings = await api.getSettings();
  const save = vi.fn().mockResolvedValue(true);
  render(<OutputPage settings={settings} onUpdateSettings={save} />);
  const shortcut = screen.getByRole("combobox", { name: "Paste shortcut" });
  expect(shortcut).toHaveValue("auto");
  fireEvent.change(shortcut, { target: { value: "ctrl_shift_v" } });
  expect(save).toHaveBeenCalledExactlyOnceWith({ paste_shortcut: "ctrl_shift_v" });
});
