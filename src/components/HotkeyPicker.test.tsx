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
  render(<HotkeyPicker value="Ctrl+K" onChange={change} />);
  fireEvent.click(screen.getByRole("button"));
  fireEvent.keyDown(window, { key: "Shift", shiftKey: true });
  fireEvent.keyDown(window, { key: "Meta", shiftKey: true, metaKey: true });
  fireEvent.keyUp(window, { key: "Shift", metaKey: true });
  expect(change).toHaveBeenCalledExactlyOnceWith("Shift+Win");
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
