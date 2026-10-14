import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { HotkeyPicker } from "./HotkeyPicker";

it("captures a key after multiple modifiers without committing prematurely", () => {
  const change = vi.fn();
  render(<HotkeyPicker value="Shift+Win" onChange={change} />);
  fireEvent.click(screen.getByRole("button"));
  fireEvent.keyDown(window, { key: "Control", ctrlKey: true });
  fireEvent.keyDown(window, { key: "Shift", ctrlKey: true, shiftKey: true });
  expect(change).not.toHaveBeenCalled();
  fireEvent.keyDown(window, { key: "k", ctrlKey: true, shiftKey: true });
  expect(change).toHaveBeenCalledExactlyOnceWith("Ctrl+Shift+K");
});

it("keeps modifier-only shortcuts by committing when a modifier is released", () => {
  const change = vi.fn();
  render(<HotkeyPicker value="Ctrl+K" onChange={change} allowModifierOnly />);
  fireEvent.click(screen.getByRole("button"));
  fireEvent.keyDown(window, { key: "Shift", shiftKey: true });
  fireEvent.keyDown(window, { key: "Meta", shiftKey: true, metaKey: true });
  fireEvent.keyUp(window, { key: "Shift", metaKey: true });
  expect(change).toHaveBeenCalledExactlyOnceWith("Shift+Win");
});

it("requires a non-modifier key on Linux and keeps capture armed", () => {
  const change = vi.fn();
  render(<HotkeyPicker value="Ctrl+K" onChange={change} allowModifierOnly={false} />);
  fireEvent.click(screen.getByRole("button"));
  fireEvent.keyDown(window, { key: "Control", ctrlKey: true });
  fireEvent.keyDown(window, { key: "Meta", ctrlKey: true, metaKey: true });
  fireEvent.keyUp(window, { key: "Meta", ctrlKey: true });
  expect(change).not.toHaveBeenCalled();
  expect(screen.getByRole("button")).toHaveTextContent("Include a key");
  fireEvent.keyDown(window, { key: " ", ctrlKey: true, shiftKey: true });
  expect(change).toHaveBeenCalledExactlyOnceWith("Ctrl+Shift+Space");
});

it("cancels capture when focus leaves the window", () => {
  const change = vi.fn();
  render(<HotkeyPicker value="Ctrl+K" onChange={change} />);
  fireEvent.click(screen.getByRole("button"));
  fireEvent.keyDown(window, { key: "Control", ctrlKey: true });
  fireEvent.blur(window);
  fireEvent.keyDown(window, { key: "a" });
  expect(change).not.toHaveBeenCalled();
  expect(screen.getByRole("button")).toHaveTextContent("Ctrl+K");
});

it("captures bare Escape for cancel actions and modified Escape for other shortcuts", () => {
  const change = vi.fn();
  const view = render(<HotkeyPicker value="Ctrl+K" onChange={change} allowEscape />);
  fireEvent.click(screen.getByRole("button"));
  fireEvent.keyDown(window, { key: "Escape" });
  expect(change).toHaveBeenCalledExactlyOnceWith("Escape");
  view.rerender(<HotkeyPicker value="Ctrl+K" onChange={change} />);
  fireEvent.click(screen.getByRole("button"));
  fireEvent.keyDown(window, { key: "Escape", ctrlKey: true });
  expect(change).toHaveBeenLastCalledWith("Ctrl+Escape");
});
