import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { api } from "../../services/tauriApi";
import type { RuntimeEntry } from "../../types";
import { RuntimeInventoryPanel } from "./RuntimeInventoryPanel";

const active: RuntimeEntry = {
  id: "current",
  version: "b9361",
  kind: "Vulkan",
  binary_path: "C:/reflow/bin/current/llama-server.exe",
  active: true,
  rollback_available: false,
  healthy: true,
  error: null,
};
const previous: RuntimeEntry = {
  ...active,
  id: "previous",
  version: "b9123",
  binary_path: "C:/reflow/bin/previous/llama-server.exe",
  active: false,
  rollback_available: true,
};

it("shows verified current and previous runtime paths and versions", async () => {
  vi.spyOn(api, "getRuntimeInventory").mockResolvedValue([active, previous]);
  render(<RuntimeInventoryPanel />);
  expect(await screen.findByText("b9361")).toBeInTheDocument();
  expect(screen.getByText(active.binary_path)).toBeInTheDocument();
  expect(screen.getByText("Current · Verified")).toBeInTheDocument();
  expect(screen.getByText("Previous · Verified")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Use previous runtime" })).toBeEnabled();
});

it("refuses rollback to an unverified previous runtime", async () => {
  vi.spyOn(api, "getRuntimeInventory").mockResolvedValue([
    active,
    { ...previous, healthy: false, error: "Digest mismatch" },
  ]);
  render(<RuntimeInventoryPanel />);
  expect(await screen.findByText("Digest mismatch")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Use previous runtime" })).not.toBeInTheDocument();
});

it("refreshes inventory after an initial failure", async () => {
  vi.spyOn(api, "getRuntimeInventory")
    .mockRejectedValueOnce(new Error("Unavailable"))
    .mockResolvedValueOnce([active]);
  render(<RuntimeInventoryPanel />);
  expect(await screen.findByRole("alert")).toHaveTextContent("Could not inspect runtimes");
  fireEvent.click(screen.getByRole("button", { name: "Refresh runtimes" }));
  expect(await screen.findByText("Current · Verified")).toBeInTheDocument();
});

it("disables other runtime operations during repair and surfaces failure", async () => {
  vi.spyOn(api, "getRuntimeInventory").mockResolvedValue([active, previous]);
  let reject!: (error: Error) => void;
  vi.spyOn(api, "repairRuntime").mockReturnValue(
    new Promise<void>((_, fail) => {
      reject = fail;
    }),
  );
  render(<RuntimeInventoryPanel />);
  await screen.findByText("Current · Verified");
  fireEvent.click(screen.getByRole("button", { name: "Repair runtime" }));
  expect(screen.getByRole("button", { name: "Repairing runtime…" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Use previous runtime" })).toBeDisabled();
  reject(new Error("Download failed"));
  expect(await screen.findByRole("alert")).toHaveTextContent("Download failed");
  expect(screen.getByRole("button", { name: "Repair runtime" })).toBeEnabled();
});

it("reloads speech after rollback and refreshes the active inventory", async () => {
  vi.spyOn(api, "getRuntimeInventory").mockResolvedValue([active, previous]);
  vi.spyOn(api, "rollbackRuntime").mockResolvedValue([
    { ...previous, active: true, rollback_available: false },
  ]);
  const reload = vi.spyOn(api, "reloadModel").mockResolvedValue(undefined);
  render(<RuntimeInventoryPanel />);
  await screen.findByText("Previous · Verified");
  fireEvent.click(screen.getByRole("button", { name: "Use previous runtime" }));
  await waitFor(() => expect(reload).toHaveBeenCalledTimes(1));
  expect(
    await screen.findByText("Previous runtime restored. Speech model reloaded."),
  ).toBeInTheDocument();
  expect(screen.queryByText("b9361")).not.toBeInTheDocument();
});

it("keeps successful rollback visible when speech reload fails and offers retry", async () => {
  vi.spyOn(api, "getRuntimeInventory").mockResolvedValue([active, previous]);
  vi.spyOn(api, "rollbackRuntime").mockResolvedValue([
    { ...previous, active: true, rollback_available: false },
  ]);
  const reload = vi
    .spyOn(api, "reloadModel")
    .mockRejectedValueOnce(new Error("ASR load failed"))
    .mockResolvedValueOnce(undefined);
  render(<RuntimeInventoryPanel />);
  await screen.findByText("Previous · Verified");
  fireEvent.click(screen.getByRole("button", { name: "Use previous runtime" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Runtime changed, but speech could not reload",
  );
  fireEvent.click(screen.getByRole("button", { name: "Reload speech model" }));
  await waitFor(() => expect(reload).toHaveBeenCalledTimes(2));
  expect(await screen.findByText("Speech model reloaded.")).toBeInTheDocument();
});
