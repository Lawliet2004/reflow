import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import * as bridge from "../services/tauriApi";
import { ShortcutProbe } from "./ShortcutProbe";

let emit: (event: { id: number; action: string; pressed: boolean }) => void;
beforeEach(() => {
  vi.spyOn(bridge, "safeListen").mockImplementation(async (_, handler) => {
    emit = handler;
    return () => {};
  });
  vi.spyOn(bridge.api, "beginShortcutCapture").mockResolvedValue(41);
  vi.spyOn(bridge.api, "endShortcutCapture").mockResolvedValue();
});

it("confirms only the owned native dictation binding without starting ASR", async () => {
  const success = vi.fn();
  const start = vi.spyOn(bridge.api, "startRecording");
  render(<ShortcutProbe onSuccess={success} />);
  fireEvent.click(screen.getByRole("button", { name: "Test shortcut" }));
  await waitFor(() => expect(bridge.api.beginShortcutCapture).toHaveBeenCalledOnce());
  act(() => {
    emit({ id: 40, action: "dictate", pressed: true });
    emit({ id: 41, action: "command", pressed: true });
  });
  expect(success).not.toHaveBeenCalled();
  act(() => emit({ id: 41, action: "dictate", pressed: true }));
  expect(success).toHaveBeenCalledOnce();
  expect(start).not.toHaveBeenCalled();
  expect(bridge.api.endShortcutCapture).not.toHaveBeenCalled();
  act(() => emit({ id: 41, action: "dictate", pressed: false }));
  expect(bridge.api.endShortcutCapture).toHaveBeenCalledWith(41);
});

it("releases a lease whose IPC reply arrives after leaving setup", async () => {
  let resolve!: (id: number) => void;
  vi.mocked(bridge.api.beginShortcutCapture).mockImplementation(
    () =>
      new Promise((yes) => {
        resolve = yes;
      }),
  );
  const success = vi.fn();
  const view = render(<ShortcutProbe onSuccess={success} />);
  fireEvent.click(screen.getByRole("button", { name: "Test shortcut" }));
  await waitFor(() => expect(resolve).toBeDefined());
  view.unmount();
  await act(async () => resolve(42));
  expect(bridge.api.endShortcutCapture).toHaveBeenCalledWith(42);
  expect(success).not.toHaveBeenCalled();
});

it("releases a pending lease when the user stops before its IPC reply", async () => {
  let resolve!: (id: number) => void;
  vi.mocked(bridge.api.beginShortcutCapture).mockImplementation(
    () =>
      new Promise((yes) => {
        resolve = yes;
      }),
  );
  const success = vi.fn();
  render(<ShortcutProbe onSuccess={success} />);
  fireEvent.click(screen.getByRole("button", { name: "Test shortcut" }));
  await waitFor(() => expect(resolve).toBeDefined());
  fireEvent.click(screen.getByRole("button", { name: "Stop shortcut test" }));
  await act(async () => resolve(43));
  expect(bridge.api.endShortcutCapture).toHaveBeenCalledWith(43);
  act(() => emit({ id: 43, action: "dictate", pressed: true }));
  expect(success).not.toHaveBeenCalled();
});
