import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { AdvancedPage } from "./AdvancedPage";
import { api } from "../../services/tauriApi";

it("lets users select audio retention separately from transcript retention", async () => {
  const settings = await api.getSettings();
  const update = vi.fn();
  render(<AdvancedPage settings={settings} onUpdateSettings={update} />);
  const select = screen.getByRole("combobox", { name: "Audio retention" });
  expect(select).toHaveValue("disabled");
  for (const policy of ["1_day", "7_days", "30_days", "forever", "disabled"]) {
    fireEvent.change(select, { target: { value: policy } });
    expect(update).toHaveBeenLastCalledWith({ audio_retention: policy });
  }
});

it("reports a failed logs command", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "openLogsFolder").mockRejectedValue(new Error("missing folder"));
  render(<AdvancedPage settings={settings} onUpdateSettings={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Open logs" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not open the logs folder");
});

it("reports diagnostics copy failures instead of leaving a silent button", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "copyDiagnostics").mockResolvedValue(false);
  render(<AdvancedPage settings={settings} onUpdateSettings={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Copy diagnostics" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not copy diagnostics");
  expect(screen.queryByRole("button", { name: "Copied" })).not.toBeInTheDocument();
});

it("explains when clipboard retry refuses to overwrite a newer user copy", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "retryClipboardRestore").mockResolvedValue(false);
  render(<AdvancedPage settings={settings} onUpdateSettings={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Retry clipboard restore" }));
  expect(await screen.findByRole("status")).toHaveTextContent(
    "No unchanged clipboard snapshot is available",
  );
});

it("reports clipboard retry failure while preserving the recovery action", async () => {
  const settings = await api.getSettings();
  vi.spyOn(api, "retryClipboardRestore").mockRejectedValue(new Error("Clipboard is busy"));
  render(<AdvancedPage settings={settings} onUpdateSettings={vi.fn()} />);
  fireEvent.click(screen.getByRole("button", { name: "Retry clipboard restore" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Clipboard is busy");
  expect(screen.getByRole("button", { name: "Retry clipboard restore" })).toBeEnabled();
});
