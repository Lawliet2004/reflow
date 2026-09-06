import { it, expect, vi } from "vitest";
import { createEventScope } from "./eventScope";
import * as bridge from "./tauriApi";

it("releases a listener that finishes registering after disposal", async () => {
  let resolve!: (fn: () => void) => void;
  const unsubscribe = vi.fn();
  vi.spyOn(bridge, "safeListen").mockImplementation(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  const scope = createEventScope();
  const pending = scope.listen("example", vi.fn());
  scope.dispose();
  resolve(unsubscribe);
  await pending;
  expect(unsubscribe).toHaveBeenCalledOnce();
});

it("does not dispatch events after disposal", async () => {
  let receive!: (payload: unknown) => void;
  vi.spyOn(bridge, "safeListen").mockImplementation(async (_event, handler) => {
    receive = handler;
    return () => {};
  });
  const scope = createEventScope();
  const callback = vi.fn();
  await scope.listen("example", callback);
  scope.dispose();
  receive("late event");
  expect(callback).not.toHaveBeenCalled();
});
