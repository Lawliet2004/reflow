import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { api } from "../../services/tauriApi";
import { APPEARANCE_DEFAULTS } from "../../types";
import { AppearancePage } from "./AppearancePage";

it("keeps the current style by default and previews the saved waveform choice in every phase", async () => {
  const settings = await api.getSettings();
  const update = vi.fn();
  const { container, rerender } = render(
    <AppearancePage settings={settings} onUpdateSettings={update} />,
  );
  expect(screen.getByRole("radio", { name: "Status pill" })).toHaveAttribute(
    "aria-checked",
    "true",
  );
  fireEvent.click(screen.getByRole("radio", { name: "Waveform pill" }));
  expect(update).toHaveBeenLastCalledWith({ hud_style: "waveform" });

  rerender(
    <AppearancePage settings={{ ...settings, hud_style: "waveform" }} onUpdateSettings={update} />,
  );
  expect(container.querySelector(".hud")).toHaveAttribute("data-style", "waveform");
  fireEvent.click(screen.getByRole("radio", { name: "Transcribing" }));
  expect(container.querySelector(".hud")).toHaveAttribute("data-phase", "transcribe");
  fireEvent.click(screen.getByRole("radio", { name: "Done" }));
  expect(container.querySelector(".capsule-check")).not.toBeNull();

  fireEvent.click(screen.getByRole("radio", { name: "Status pill" }));
  expect(update).toHaveBeenLastCalledWith({ hud_style: "status" });
  fireEvent.click(screen.getByRole("button", { name: "Reset appearance" }));
  fireEvent.click(screen.getByRole("button", { name: "Reset appearance" }));
  expect(update).toHaveBeenLastCalledWith(APPEARANCE_DEFAULTS);
  expect(APPEARANCE_DEFAULTS.hud_style).toBe("status");
});
