import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { HotkeyPicker } from "./HotkeyPicker";

it("captures a key after multiple modifiers without committing prematurely", async () => {
  const change = vi.fn();
  render(<HotkeyPicker value="Shift+Win" onChange={change} />);
  fireEvent.click(screen.getByRole("button"), { detail: 1 });
  await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("Press keys"));
  fireEvent.keyDown(window, { key: "Control", ctrlKey: true });
  fireEvent.keyDown(window, { key: "Shift", ctrlKey: true, shiftKey: true });
  expect(change).not.toHaveBeenCalled();
  fireEvent.keyDown(window, { key: "k", ctrlKey: true, shiftKey: true });
  expect(change).toHaveBeenCalledExactlyOnceWith("Ctrl+Shift+K");
});

it("keeps modifier-only shortcuts by committing when a modifier is released", async () => {
  const change = vi.fn();
  render(<HotkeyPicker value="Ctrl+K" onChange={change} allowModifierOnly />);
  fireEvent.click(screen.getByRole("button"), { detail: 1 });
  await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("Press keys"));
  fireEvent.keyDown(window, { key: "Shift", shiftKey: true });
  fireEvent.keyDown(window, { key: "Meta", shiftKey: true, metaKey: true });
  fireEvent.keyUp(window, { key: "Shift", metaKey: true });
  expect(change).toHaveBeenCalledExactlyOnceWith("Shift+Win");
});

it("requires a regular key when modifier-only capture is disabled", async () => {
  const change = vi.fn();
  render(<HotkeyPicker value="Ctrl+K" onChange={change} allowModifierOnly={false} />);
  fireEvent.click(screen.getByRole("button"), { detail: 1 });
  await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("Press keys"));
  fireEvent.keyDown(window, { key: "Control", ctrlKey: true });
  fireEvent.keyDown(window, { key: "Meta", ctrlKey: true, metaKey: true });
  fireEvent.keyUp(window, { key: "Meta", ctrlKey: true });
  expect(change).not.toHaveBeenCalled();
  await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("Include a key"));
  fireEvent.keyDown(window, { key: " ", ctrlKey: true, shiftKey: true });
  expect(change).toHaveBeenCalledExactlyOnceWith("Ctrl+Shift+Space");
});

it("cancels capture when focus leaves the window", async () => {
  const change = vi.fn();
  render(<HotkeyPicker value="Ctrl+K" onChange={change} />);
  fireEvent.click(screen.getByRole("button"), { detail: 1 });
  await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("Press keys"));
  fireEvent.keyDown(window, { key: "Control", ctrlKey: true });
  fireEvent.blur(window);
  fireEvent.keyDown(window, { key: "a" });
  expect(change).not.toHaveBeenCalled();
  expect(screen.getByRole("button")).toHaveTextContent("Ctrl+K");
});

it("captures bare Escape for cancel actions and modified Escape for other shortcuts", async () => {
  const change = vi.fn();
  const view = render(<HotkeyPicker value="Ctrl+K" onChange={change} allowEscape />);
  fireEvent.click(screen.getByRole("button"), { detail: 1 });
  await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("Press keys"));
  fireEvent.keyDown(window, { key: "Escape" });
  expect(change).toHaveBeenCalledExactlyOnceWith("Escape");
  view.rerender(<HotkeyPicker value="Ctrl+K" onChange={change} />);
  fireEvent.click(screen.getByRole("button"), { detail: 1 });
  await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("Press keys"));
  fireEvent.keyDown(window, { key: "Escape", ctrlKey: true });
  expect(change).toHaveBeenLastCalledWith("Ctrl+Escape");
});

it("keeps listening when clicked again instead of toggling out of capture", async () => {
  const change = vi.fn();
  render(<HotkeyPicker value="Ctrl+Space" onChange={change} />);
  fireEvent.click(screen.getByRole("button"), { detail: 1 });
  await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("Press keys"));
  fireEvent.click(screen.getByRole("button"), { detail: 1 });
  expect(screen.getByRole("button")).toHaveTextContent("Press keys");
  fireEvent.keyDown(window, { key: "k", ctrlKey: true });
  expect(change).toHaveBeenCalledExactlyOnceWith("Ctrl+K");
});

it("captures a combination swallowed by the desktop global listener", async () => {
  const { api } = await import("../services/tauriApi");
  const bridge = await import("../services/tauriApi");
  vi.spyOn(api, "beginShortcutCapture").mockResolvedValue(23);
  const end = vi.spyOn(api, "endShortcutCapture").mockResolvedValue();
  let emit!: (value: { id: number; shortcut: string; pressed: boolean }) => void;
  vi.spyOn(bridge, "safeListen").mockImplementation(async (_event, handler) => {
    emit = handler;
    return () => {};
  });
  const change = vi.fn();
  render(<HotkeyPicker value="Ctrl+Space" onChange={change} />);
  fireEvent.click(screen.getByRole("button"), { detail: 1 });
  await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("Press keys"));
  await act(async () => emit({ id: 22, shortcut: "Ctrl+Space", pressed: true }));
  expect(change).not.toHaveBeenCalled();
  await act(async () => emit({ id: 23, shortcut: "Ctrl+Space", pressed: true }));
  expect(change).toHaveBeenCalledExactlyOnceWith("Ctrl+Space");
  await act(async () => emit({ id: 23, shortcut: "Ctrl+Space", pressed: false }));
  expect(end).toHaveBeenCalledExactlyOnceWith(23);
});

it("does not rearm from the keyboard-generated click after capturing Space", async () => {
  render(<HotkeyPicker value="Ctrl+K" onChange={vi.fn()} />);
  fireEvent.click(screen.getByRole("button"), { detail: 1 });
  await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("Press keys"));
  fireEvent.keyDown(window, { key: " ", ctrlKey: true });
  fireEvent.keyUp(window, { key: " ", ctrlKey: true });
  fireEvent.click(screen.getByRole("button"), { detail: 0 });
  expect(screen.getByRole("button")).not.toHaveTextContent("Press keys");
});

it("stops capture if a pending native acknowledgement arrives after unmount", async () => {
  const { api } = await import("../services/tauriApi");
  let finish!: (id: number) => void;
  const begin = vi.spyOn(api, "beginShortcutCapture").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const end = vi.spyOn(api, "endShortcutCapture").mockResolvedValue();
  const view = render(<HotkeyPicker value="Ctrl+K" onChange={vi.fn()} />);
  fireEvent.click(screen.getByRole("button"), { detail: 1 });
  await waitFor(() => expect(begin).toHaveBeenCalledOnce());
  view.unmount();
  await act(async () => finish(91));
  expect(end).toHaveBeenCalledExactlyOnceWith(91);
});

it.each([
  {
    keys: ["Control", "Shift"],
    first: { ctrlKey: true },
    both: { ctrlKey: true, shiftKey: true },
    remaining: { ctrlKey: true },
    expected: "Ctrl+Shift",
  },
  {
    keys: ["Control", "Meta"],
    first: { ctrlKey: true },
    both: { ctrlKey: true, metaKey: true },
    remaining: { ctrlKey: true },
    expected: "Ctrl+Win",
  },
  {
    keys: ["Shift", "Meta"],
    first: { shiftKey: true },
    both: { shiftKey: true, metaKey: true },
    remaining: { shiftKey: true },
    expected: "Shift+Win",
  },
])(
  "captures $expected on desktops with modifier-only support",
  async ({ keys, first, both, remaining, expected }) => {
    const change = vi.fn();
    render(<HotkeyPicker value="Ctrl+Space" onChange={change} allowModifierOnly />);
    fireEvent.click(screen.getByRole("button"), { detail: 1 });
    await waitFor(() => expect(screen.getByRole("button")).toHaveTextContent("Press keys"));
    fireEvent.keyDown(window, { key: keys[0], ...first });
    fireEvent.keyDown(window, { key: keys[1], ...both });
    expect(change).not.toHaveBeenCalled();
    fireEvent.keyUp(window, { key: keys[1], ...remaining });
    expect(change).toHaveBeenCalledExactlyOnceWith(expected);
  },
);
