import { fireEvent, renderHook } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { useUiZoom } from "./useUiZoom";

it("steps zoom with Ctrl+= and Ctrl+- and resets with Ctrl+0", () => {
  const update = vi.fn();
  const view = renderHook(({ zoom }) => useUiZoom(zoom, update), {
    initialProps: { zoom: 100 },
  });
  fireEvent.keyDown(window, { code: "Equal", ctrlKey: true });
  expect(update).toHaveBeenLastCalledWith({ ui_zoom: 110 });
  view.rerender({ zoom: 110 });
  fireEvent.keyDown(window, { code: "Minus", ctrlKey: true });
  expect(update).toHaveBeenLastCalledWith({ ui_zoom: 100 });
  view.rerender({ zoom: 140 });
  fireEvent.keyDown(window, { code: "Digit0", ctrlKey: true });
  expect(update).toHaveBeenLastCalledWith({ ui_zoom: 100 });
});

it("ignores zoom keys without a modifier and clamps at the bounds", () => {
  const update = vi.fn();
  const view = renderHook(({ zoom }) => useUiZoom(zoom, update), {
    initialProps: { zoom: 100 },
  });
  fireEvent.keyDown(window, { code: "Equal" });
  fireEvent.keyDown(window, { code: "Equal", altKey: true });
  expect(update).not.toHaveBeenCalled();
  view.rerender({ zoom: 160 });
  fireEvent.keyDown(window, { code: "Equal", ctrlKey: true });
  expect(update).not.toHaveBeenCalled();
  view.rerender({ zoom: 80 });
  fireEvent.keyDown(window, { code: "Minus", ctrlKey: true });
  expect(update).not.toHaveBeenCalled();
});

it("leaves already-claimed keys to other controls", () => {
  const update = vi.fn();
  renderHook(() => useUiZoom(100, update));
  const event = new KeyboardEvent("keydown", {
    code: "Equal",
    ctrlKey: true,
    cancelable: true,
  });
  event.preventDefault();
  window.dispatchEvent(event);
  expect(update).not.toHaveBeenCalled();
});

it("follows Ctrl+scroll and accumulates small wheel deltas", () => {
  const update = vi.fn();
  const view = renderHook(({ zoom }) => useUiZoom(zoom, update), {
    initialProps: { zoom: 100 },
  });
  fireEvent.wheel(window, { deltaY: -20, ctrlKey: true });
  fireEvent.wheel(window, { deltaY: -20, ctrlKey: true });
  expect(update).not.toHaveBeenCalled();
  fireEvent.wheel(window, { deltaY: -20, ctrlKey: true });
  expect(update).toHaveBeenLastCalledWith({ ui_zoom: 110 });
  view.rerender({ zoom: 110 });
  fireEvent.wheel(window, { deltaY: 120 });
  expect(update).toHaveBeenCalledTimes(1);
  fireEvent.wheel(window, { deltaY: 120, ctrlKey: true });
  expect(update).toHaveBeenLastCalledWith({ ui_zoom: 100 });
});
