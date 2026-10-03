import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { useState } from "react";
import userEvent from "@testing-library/user-event";
import { api } from "../../services/tauriApi";
import { ExpansionAdvanced } from "./ExpansionAdvanced";

it("preserves line breaks while entering multiple excluded applications", async () => {
  const initial = await api.getSettings();
  const save = vi.fn().mockResolvedValue(true);
  function Settings() {
    const [settings, setSettings] = useState(initial);
    return (
      <ExpansionAdvanced
        settings={settings}
        onUpdateSettings={async (patch) => {
          save(patch);
          setSettings((current) => ({ ...current, ...patch }));
          return true;
        }}
      />
    );
  }
  render(<Settings />);
  const input = screen.getByRole("textbox", { name: /Excluded application/ });
  await userEvent.setup().type(input, "code.exe{Enter}browser.exe");
  expect(input).toHaveValue("code.exe\nbrowser.exe");
  expect(save).not.toHaveBeenCalled();
  fireEvent.blur(input);
  await waitFor(() =>
    expect(save).toHaveBeenCalledWith({ excluded_apps: ["code.exe", "browser.exe"] }),
  );
});

it("shows airplane mode accurately and persists an explicit download toggle", async () => {
  const settings = await api.getSettings();
  const save = vi.fn();
  render(
    <ExpansionAdvanced settings={{ ...settings, offline_mode: true }} onUpdateSettings={save} />,
  );
  const toggle = screen.getByRole("switch", { name: "Airplane mode" });
  expect(toggle).toHaveAttribute("aria-checked", "true");
  fireEvent.click(toggle);
  expect(save).toHaveBeenCalledWith({ offline_mode: false });
  expect(await screen.findByText("0 characters")).toBeInTheDocument();
});

it("does not create an automation credential merely by opening settings", async () => {
  const create = vi.spyOn(api, "createAutomationToken");
  render(
    <ExpansionAdvanced
      settings={{ ...(await api.getSettings()), api_enabled: false }}
      onUpdateSettings={vi.fn()}
    />,
  );
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Create or rotate token" })).toBeDisabled(),
  );
  expect(create).not.toHaveBeenCalled();
});
